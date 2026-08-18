//! TLS upgrade over an RDPEUDP2 stream.
//!
//! Mirrors the rustls configuration from `ironrdp-tls`: no certificate
//! verification (RDP uses self-signed certs), disabled TLS resumption
//! (CredSSP compatibility), and SSLKEYLOGFILE support for Wireshark.
//!
//! The caller passes an `RdpeudpStream` which provides `AsyncRead +
//! AsyncWrite` over the RDPEUDP2 connection. tokio-rustls wraps it
//! transparently: the TLS layer doesn't know it's running over UDP.

use std::io;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio_rustls::rustls::pki_types::ServerName;

pub(crate) type TlsStream<S> = tokio_rustls::client::TlsStream<S>;
pub(crate) type ServerTlsStream<S> = tokio_rustls::server::TlsStream<S>;

/// Perform TLS handshake over an established RDPEUDP2 stream.
///
/// `server_cert_verifier` selects how the peer certificate is validated.
/// `None` preserves the historic behavior (no verification: RDP servers
/// commonly present self-signed certificates). `Some(verifier)` lets a
/// caller opt into real validation, for example a
/// `rustls::client::WebPkiServerVerifier` built against the platform root
/// store, mirroring the choice `ironrdp-tls` already exposes on the TCP
/// path without this crate taking on that crate's certificate-store
/// dependencies itself.
///
/// Returns the encrypted stream. The driver task continues running
/// in the background, transparently shuttling encrypted bytes between
/// the UDP socket and this TLS stream via `SharedIo`.
pub(crate) async fn tls_upgrade<S>(
    stream: S,
    server_name: &str,
    server_cert_verifier: Option<Arc<dyn tokio_rustls::rustls::client::danger::ServerCertVerifier>>,
) -> io::Result<TlsStream<S>>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let verifier: Arc<dyn tokio_rustls::rustls::client::danger::ServerCertVerifier> =
        server_cert_verifier.unwrap_or_else(|| Arc::new(danger::NoCertificateVerification));

    let mut tls_stream = {
        let mut config = tokio_rustls::rustls::client::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();

        // SSLKEYLOGFILE support for debugging with Wireshark
        config.key_log = Arc::new(tokio_rustls::rustls::KeyLogFile::new());

        // Disable TLS resumption: not supported by CredSSP and some
        // RDP server configurations.
        config.resumption = tokio_rustls::rustls::client::Resumption::disabled();

        let config = Arc::new(config);

        let domain = ServerName::try_from(server_name.to_owned()).map_err(io::Error::other)?;

        tokio_rustls::TlsConnector::from(config).connect(domain, stream).await?
    };

    tls_stream.flush().await?;

    Ok(tls_stream)
}

/// Perform TLS server-side handshake over an established RDPEUDP2 stream.
///
/// The caller provides a `ServerConfig` with certificate and private key.
/// Returns the encrypted stream for the RDPEMT tunnel handshake.
pub(crate) async fn tls_accept<S>(
    stream: S,
    config: Arc<tokio_rustls::rustls::ServerConfig>,
) -> io::Result<ServerTlsStream<S>>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    let mut tls_stream = acceptor.accept(stream).await?;
    tls_stream.flush().await?;
    Ok(tls_stream)
}

mod danger {
    use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use tokio_rustls::rustls::{DigitallySignedStruct, Error, SignatureScheme, pki_types};

    #[derive(Debug)]
    pub(super) struct NoCertificateVerification;

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _: &pki_types::CertificateDer<'_>,
            _: &[pki_types::CertificateDer<'_>],
            _: &pki_types::ServerName<'_>,
            _: &[u8],
            _: pki_types::UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA1,
                SignatureScheme::ECDSA_SHA1_Legacy,
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP521_SHA512,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
                SignatureScheme::ED448,
            ]
        }
    }
}
