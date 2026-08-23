use std::io;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

use crate::{CertificateValidation, CertificateValidationCallback};

pub type TlsStream<S> = tokio_native_tls::TlsStream<S>;

pub async fn upgrade<S>(stream: S, server_name: &str) -> io::Result<(TlsStream<S>, x509_cert::Certificate)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    upgrade_with_certificate_validation(stream, server_name, CertificateValidation::default()).await
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
        let mut builder = tokio_native_tls::native_tls::TlsConnector::builder();
        if certificate_validation == CertificateValidation::DangerouslyAcceptInvalidCertificate {
            builder.danger_accept_invalid_certs(true);
            builder.danger_accept_invalid_hostnames(true);
            builder.use_sni(false);
        }
        let connector = builder
            .build()
            .map(tokio_native_tls::TlsConnector::from)
            .map_err(io::Error::other)?;

        connector.connect(server_name, stream).await.map_err(io::Error::other)?
    };

    tls_stream.flush().await?;

    let tls_cert = {
        use x509_cert::der::Decode as _;

        let cert = tls_stream
            .get_ref()
            .peer_certificate()
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("peer certificate is missing"))?;
        let cert = cert.to_der().map_err(io::Error::other)?;

        x509_cert::Certificate::from_der(&cert).map_err(io::Error::other)?
    };

    Ok((tls_stream, tls_cert))
}

/// The `native-tls` backend cannot safely expose a certificate-validation callback.
pub async fn upgrade_with_certificate_validation_callback<S>(
    stream: S,
    server_name: &str,
    callback: CertificateValidationCallback,
) -> io::Result<(TlsStream<S>, x509_cert::Certificate)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let _ = (stream, server_name, callback);
    Err(io::Error::other(
        "certificate validation callbacks require the rustls backend",
    ))
}

/// The `native-tls` backend cannot safely expose a certificate-validation callback.
pub async fn upgrade_with_certificate_validation_callback_for_endpoint<S>(
    stream: S,
    server_name: &str,
    endpoint: &str,
    callback: CertificateValidationCallback,
) -> io::Result<(TlsStream<S>, x509_cert::Certificate)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let _ = endpoint;
    upgrade_with_certificate_validation_callback(stream, server_name, callback).await
}

/// The `native-tls` backend does not expose the negotiated version or cipher.
pub fn negotiated<S>(_stream: &TlsStream<S>) -> crate::NegotiatedTls {
    crate::NegotiatedTls::default()
}
