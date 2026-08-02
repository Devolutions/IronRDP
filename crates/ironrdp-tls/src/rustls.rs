use std::io;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio_rustls::rustls;
use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use tokio_rustls::rustls::pki_types::ServerName;

use crate::{CertificateValidation, CertificateValidationCallback};

pub type TlsStream<S> = tokio_rustls::client::TlsStream<S>;

pub async fn upgrade<S>(stream: S, server_name: &str) -> io::Result<(TlsStream<S>, x509_cert::Certificate)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    upgrade_with_certificate_validation(stream, server_name, CertificateValidation::Strict).await
}

/// Upgrades `stream` to TLS using the explicitly selected certificate-validation policy.
///
/// The dangerous policy is intended only for controlled development and test environments.
pub async fn upgrade_with_certificate_validation<S>(
    stream: S,
    server_name: &str,
    certificate_validation: CertificateValidation,
) -> io::Result<(TlsStream<S>, x509_cert::Certificate)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let mut tls_stream = {
        let mut config = match certificate_validation {
            CertificateValidation::Strict => rustls::client::ClientConfig::builder()
                .with_root_certificates(platform_root_certificates()?)
                .with_no_client_auth(),
            CertificateValidation::DangerouslyAcceptInvalidCertificate => rustls::client::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(danger::NoCertificateVerification))
                .with_no_client_auth(),
        };

        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        config.key_log = Arc::new(rustls::KeyLogFile::new());

        // Disable TLS resumption because it’s not supported by some services such as CredSSP.
        //
        // > The CredSSP Protocol does not extend the TLS wire protocol. TLS session resumption is not supported.
        //
        // source: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cssp/385a7489-d46b-464c-b224-f7340e308a5c
        config.resumption = rustls::client::Resumption::disabled();

        let config = Arc::new(config);

        let domain = ServerName::try_from(server_name.to_owned()).map_err(io::Error::other)?;

        tokio_rustls::TlsConnector::from(config).connect(domain, stream).await?
    };

    tls_stream.flush().await?;

    let tls_cert = {
        use x509_cert::der::Decode as _;

        let cert = tls_stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certificates| certificates.first())
            .ok_or_else(|| io::Error::other("peer certificate is missing"))?;

        x509_cert::Certificate::from_der(cert).map_err(io::Error::other)?
    };

    Ok((tls_stream, tls_cert))
}

/// Upgrades a stream with normal platform-root and server-name validation.
///
/// If validation fails, `callback` is invoked synchronously on the handshake thread
/// with the leaf certificate and validation error. A callback approval accepts that
/// certificate for this handshake only.
pub async fn upgrade_with_certificate_validation_callback<S>(
    stream: S,
    server_name: &str,
    callback: CertificateValidationCallback,
) -> io::Result<(TlsStream<S>, x509_cert::Certificate)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let verifier = rustls::client::WebPkiServerVerifier::builder(Arc::new(platform_root_certificates()?))
        .build()
        .map_err(io::Error::other)?;
    let verifier: Arc<dyn ServerCertVerifier> = verifier;
    let mut config = rustls::client::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(CallbackVerifier { verifier, callback }))
        .with_no_client_auth();
    config.key_log = Arc::new(rustls::KeyLogFile::new());
    // TLS resumption is incompatible with CredSSP.
    config.resumption = rustls::client::Resumption::disabled();

    let domain = ServerName::try_from(server_name.to_owned()).map_err(io::Error::other)?;
    let mut tls_stream = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(domain, stream)
        .await?;
    tls_stream.flush().await?;

    let tls_cert = {
        use x509_cert::der::Decode as _;

        let cert = tls_stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certificates| certificates.first())
            .ok_or_else(|| io::Error::other("peer certificate is missing"))?;

        x509_cert::Certificate::from_der(cert).map_err(io::Error::other)?
    };

    Ok((tls_stream, tls_cert))
}

/// Report the TLS version and cipher suite negotiated for `stream`.
pub fn negotiated<S>(stream: &TlsStream<S>) -> crate::NegotiatedTls {
    let (_, connection) = stream.get_ref();
    crate::NegotiatedTls {
        version: connection.protocol_version().map(|version| format!("{version:?}")),
        cipher_suite: connection
            .negotiated_cipher_suite()
            .map(|suite| format!("{:?}", suite.suite())),
    }
}

fn platform_root_certificates() -> io::Result<rustls::RootCertStore> {
    let native_certificates = rustls_native_certs::load_native_certs();
    let mut roots = rustls::RootCertStore::empty();
    let (added, _) = roots.add_parsable_certificates(native_certificates.certs);
    if added == 0 {
        return Err(io::Error::other(
            "the platform certificate store contains no usable roots",
        ));
    }

    Ok(roots)
}

struct CallbackVerifier {
    verifier: Arc<dyn ServerCertVerifier>,
    callback: CertificateValidationCallback,
}

impl core::fmt::Debug for CallbackVerifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CallbackVerifier")
    }
}

impl ServerCertVerifier for CallbackVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self
            .verifier
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            Ok(verified) => Ok(verified),
            Err(error) if (self.callback)(end_entity.as_ref(), &error.to_string()) => {
                Ok(ServerCertVerified::assertion())
            }
            Err(error) => Err(error),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.verifier.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.verifier.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.verifier.supported_verify_schemes()
    }
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
