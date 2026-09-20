//! TLS 1.3 with self-signed certificates on both sides. The verifiers pin
//! nothing at the handshake: they record which certificate was presented
//! and let the connection layer decide, because an unpaired peer is still
//! allowed to complete a handshake in order to pair.

use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{
    ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms,
};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme,
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::net::TcpStream;

use crate::identity::Identity;

/// Answers whether a fingerprint belongs to a paired peer.
pub type Trusted = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The name the client asks for; the verifier ignores it.
pub const SERVER_NAME: &str = "owl-transfer";

struct PinVerifier {
    trusted: Trusted,
    algs: WebPkiSupportedAlgorithms,
}

impl std::fmt::Debug for PinVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PinVerifier")
    }
}

impl PinVerifier {
    fn note(&self, cert: &CertificateDer<'_>) {
        let fp = Identity::fingerprint(cert.as_ref());
        if !(self.trusted)(&fp) {
            tracing::debug!("handshake with unpaired certificate {fp}");
        }
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.note(end_entity);
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

impl ClientCertVerifier for PinVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.note(end_entity);
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

pub fn server_config(identity: &Identity, trusted: Trusted) -> Result<Arc<ServerConfig>> {
    let provider = provider();
    let verifier = Arc::new(PinVerifier {
        trusted,
        algs: provider.signature_verification_algorithms,
    });
    let config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("TLS 1.3 unavailable")?
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![identity.cert()], identity.key())
        .context("building the server TLS config")?;
    Ok(Arc::new(config))
}

pub fn client_config(identity: &Identity, trusted: Trusted) -> Result<Arc<ClientConfig>> {
    let provider = provider();
    let verifier = Arc::new(PinVerifier {
        trusted,
        algs: provider.signature_verification_algorithms,
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("TLS 1.3 unavailable")?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![identity.cert()], identity.key())
        .context("building the client TLS config")?;
    Ok(Arc::new(config))
}

/// The fingerprint of the certificate the other side presented, for either
/// side of a finished handshake.
pub fn peer_fingerprint(stream: &tokio_rustls::TlsStream<TcpStream>) -> Option<String> {
    let (_, common) = stream.get_ref();
    let cert = common.peer_certificates()?.first()?;
    Some(Identity::fingerprint(cert.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    #[tokio::test]
    async fn two_identities_handshake_over_localhost() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Identity::load_or_create(dir_a.path()).unwrap();
        let b = Identity::load_or_create(dir_b.path()).unwrap();
        let accept_all: Trusted = Arc::new(|_| true);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = TlsAcceptor::from(server_config(&a, accept_all.clone()).unwrap());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let stream = acceptor.accept(tcp).await.unwrap();
            peer_fingerprint(&tokio_rustls::TlsStream::Server(stream))
        });

        let connector = TlsConnector::from(client_config(&b, accept_all).unwrap());
        let tcp = TcpStream::connect(addr).await.unwrap();
        let name = ServerName::try_from(SERVER_NAME).unwrap();
        let stream = connector.connect(name, tcp).await.unwrap();
        let seen_by_client = peer_fingerprint(&tokio_rustls::TlsStream::Client(stream));

        let seen_by_server = server.await.unwrap();
        assert_eq!(seen_by_server.as_deref(), Some(b.id.as_str()));
        assert_eq!(seen_by_client.as_deref(), Some(a.id.as_str()));
    }
}
