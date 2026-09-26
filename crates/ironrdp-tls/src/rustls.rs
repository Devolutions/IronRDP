use std::io;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio_rustls::rustls::pki_types::ServerName;

use crate::{CertificateValidation, CertificateValidationCallback};

pub type TlsStream<S> = tokio_rustls::client::TlsStream<S>;

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
        let config = Arc::new(crate::rustls_client_config(certificate_validation, server_name, None)?);

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
    upgrade_with_certificate_validation_callback_for_endpoint(stream, server_name, server_name, callback).await
}

/// Upgrades a stream with normal platform-root and server-name validation.
///
/// On validation failure, invokes `callback` with `endpoint` so callers can scope
/// certificate exceptions to the configured connection endpoint.
pub async fn upgrade_with_certificate_validation_callback_for_endpoint<S>(
    stream: S,
    server_name: &str,
    endpoint: &str,
    callback: CertificateValidationCallback,
) -> io::Result<(TlsStream<S>, x509_cert::Certificate)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let config = crate::rustls_client_config(CertificateValidation::Strict, endpoint, Some(callback))?;
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
