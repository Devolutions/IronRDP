use std::io;
use std::sync::Arc;

use tokio_rustls::rustls;
use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use tokio_rustls::rustls::pki_types::ServerName;

use crate::{CertificateValidation, CertificateValidationCallback};

/// Builds the rustls client configuration used by an RDP transport.
///
/// A callback augments strict platform-root and server-name validation.
/// Combining a callback with the dangerous policy is rejected because the callback would never observe a validation failure.
pub fn rustls_client_config(
    certificate_validation: CertificateValidation,
    endpoint: &str,
    callback: Option<CertificateValidationCallback>,
) -> io::Result<rustls::ClientConfig> {
    let verifier = server_cert_verifier(certificate_validation, endpoint, callback)?;
    let mut config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.key_log = Arc::new(rustls::KeyLogFile::new());
    config.resumption = rustls::client::Resumption::disabled();
    Ok(config)
}

fn server_cert_verifier(
    certificate_validation: CertificateValidation,
    endpoint: &str,
    callback: Option<CertificateValidationCallback>,
) -> io::Result<Arc<dyn ServerCertVerifier>> {
    if callback.is_some() && certificate_validation != CertificateValidation::Strict {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a certificate-validation callback requires strict validation",
        ));
    }

    let verifier: Arc<dyn ServerCertVerifier> = match certificate_validation {
        CertificateValidation::Strict => {
            rustls::client::WebPkiServerVerifier::builder(Arc::new(platform_root_certificates()?))
                .build()
                .map_err(io::Error::other)?
        }
        CertificateValidation::DangerouslyAcceptInvalidCertificate => Arc::new(NoCertificateVerification),
    };

    Ok(match callback {
        Some(callback) => Arc::new(CallbackVerifier {
            verifier,
            endpoint: endpoint.to_owned(),
            callback,
        }),
        None => verifier,
    })
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
    endpoint: String,
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
            Err(error) if (self.callback)(end_entity.as_ref(), &self.endpoint, &error.to_string()) => {
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

#[derive(Debug)]
struct NoCertificateVerification;

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA1,
            rustls::SignatureScheme::ECDSA_SHA1_Legacy,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}
