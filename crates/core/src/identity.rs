//! The device identity: a self-signed Ed25519 certificate whose SHA-256
//! fingerprint is the device id that peers pin.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use rcgen::{CertificateParams, KeyPair};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::atomic::write_atomic;

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
            // Damage here is fatal on purpose: a fresh identity would be a
            // different device to every peer, so the person must decide.
            let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let parsed = serde_json::from_slice::<DeviceFile>(&raw)
                .map_err(anyhow::Error::from)
                .and_then(|file| Identity::from_pem(&file.cert_pem, &file.key_pem));
            return match parsed {
                Ok(identity) => Ok(identity),
                Err(e) => bail!(
                    "the device identity at {} is unreadable ({e:#}). Restore it from a \
                     backup, or delete it to create a new identity; every paired device \
                     will then need pairing again.",
                    path.display()
                ),
            };
        }

        let key = KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
        let cert = CertificateParams::new(vec![CERT_NAME.to_string()])?.self_signed(&key)?;
        let file = DeviceFile {
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
        };
        write_atomic(&path, &serde_json::to_vec_pretty(&file)?, true)?;
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
    fn a_damaged_device_file_is_fatal_with_a_clear_message() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("device.json"), b"").unwrap();
        let err = Identity::load_or_create(dir.path())
            .unwrap_err()
            .to_string();
        assert!(err.contains("pairing again"), "{err}");
        assert!(
            dir.path().join("device.json").exists(),
            "nothing is moved or replaced"
        );
    }

    #[cfg(unix)]
    #[test]
    fn device_file_is_private_from_the_start() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        Identity::load_or_create(dir.path()).unwrap();
        let mode = fs::metadata(dir.path().join("device.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn fingerprint_matches_sha256() {
        assert_eq!(
            Identity::fingerprint(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
