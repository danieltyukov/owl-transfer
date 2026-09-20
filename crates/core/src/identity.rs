//! The device identity: a self-signed Ed25519 certificate whose SHA-256
//! fingerprint is the device id that peers pin.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rcgen::{CertificateParams, KeyPair};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DEVICE_FILE: &str = "device.json";
const CERT_NAME: &str = "owl-transfer";

#[derive(Clone)]
pub struct Identity {
    /// Lowercase hex SHA-256 of `cert_der`, 64 characters.
    pub id: String,
    pub cert_der: Vec<u8>,
    /// PKCS#8 DER.
    pub key_der: Vec<u8>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("id", &self.id).finish()
    }
}

#[derive(Serialize, Deserialize)]
struct DeviceFile {
    cert_pem: String,
    key_pem: String,
}

impl Identity {
    /// Loads `data_dir/device.json` or generates a new identity and writes it.
    pub fn load_or_create(data_dir: &Path) -> Result<Identity> {
        fs::create_dir_all(data_dir).with_context(|| format!("creating {}", data_dir.display()))?;
        let path = data_dir.join(DEVICE_FILE);
        if path.exists() {
            let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let file: DeviceFile = serde_json::from_slice(&raw)
                .with_context(|| format!("parsing {}", path.display()))?;
            return Identity::from_pem(&file.cert_pem, &file.key_pem);
        }

        let key = KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
        let cert = CertificateParams::new(vec![CERT_NAME.to_string()])?.self_signed(&key)?;
        let file = DeviceFile {
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
        };
        write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
        Identity::from_pem(&file.cert_pem, &file.key_pem)
    }

    fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Identity> {
        let cert = CertificateDer::from_pem_slice(cert_pem.as_bytes())
            .context("device.json holds an invalid certificate")?;
        let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
            .context("device.json holds an invalid private key")?;
        let cert_der = cert.as_ref().to_vec();
        Ok(Identity {
            id: Identity::fingerprint(&cert_der),
            cert_der,
            key_der: key.secret_der().to_vec(),
        })
    }

    /// Lowercase hex SHA-256 of a DER certificate.
    pub fn fingerprint(cert_der: &[u8]) -> String {
        hex::encode(Sha256::digest(cert_der))
    }

    pub fn cert(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert_der.clone())
    }

    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone()))
    }
}

/// Writes the file through a temporary name so a crash never leaves a half
/// written identity, and keeps it readable by the owner only where the
/// platform has such a notion.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_load_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let first = Identity::load_or_create(dir.path()).unwrap();
        let second = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.id.len(), 64);
        assert!(first
            .id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert!(dir.path().join("device.json").exists());
        assert_eq!(first.cert_der, second.cert_der);
        assert_eq!(first.key_der, second.key_der);
    }

    #[test]
    fn fingerprint_matches_sha256() {
        assert_eq!(
            Identity::fingerprint(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
