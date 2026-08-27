//! TLS upgrade over an RDPEUDP2 stream.
//!
//! Reuses the `ironrdp-tls` client configuration.
//!
//! The caller passes an `RdpeudpStream` which provides `AsyncRead +
//! AsyncWrite` over the RDPEUDP2 connection. tokio-rustls wraps it
//! transparently: the TLS layer doesn't know it's running over UDP.

use std::io;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio_rustls::rustls::pki_types::ServerName;

use ironrdp_tls::{CertificateValidation, CertificateValidationCallback};

pub(crate) type TlsStream<S> = tokio_rustls::client::TlsStream<S>;
pub(crate) type ServerTlsStream<S> = tokio_rustls::server::TlsStream<S>;

/// Perform TLS handshake over an established RDPEUDP2 stream.
///
/// The policy, callback, and endpoint use the same semantics as `ironrdp-tls` on the primary TCP transport.
///
/// A synchronous validation callback runs on a dedicated blocking thread so it cannot starve the RDPEUDP driver on a current-thread runtime.
///
/// Returns the encrypted stream.
/// The driver task continues running in the background, transparently shuttling encrypted bytes between the UDP socket and this TLS stream via `SharedIo`.
pub(crate) async fn tls_upgrade<S>(
    stream: S,
    server_name: &str,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
    certificate_validation_endpoint: &str,
) -> io::Result<TlsStream<S>>
where
    S: Unpin + Send + AsyncRead + AsyncWrite + 'static,
{
    if certificate_validation_callback.is_none() {
        return tls_upgrade_inner(
            stream,
            server_name,
            certificate_validation,
            None,
            certificate_validation_endpoint,
        )
        .await;
    }

    let server_name = server_name.to_owned();
    let certificate_validation_endpoint = certificate_validation_endpoint.to_owned();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(tls_upgrade_inner(
                stream,
                &server_name,
                certificate_validation,
                certificate_validation_callback,
                &certificate_validation_endpoint,
            ))
    })
    .await
    .map_err(io::Error::other)?
}

async fn tls_upgrade_inner<S>(
    stream: S,
    server_name: &str,
    certificate_validation: CertificateValidation,
    certificate_validation_callback: Option<CertificateValidationCallback>,
    certificate_validation_endpoint: &str,
) -> io::Result<TlsStream<S>>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let config = ironrdp_tls::rustls_client_config(
        certificate_validation,
        certificate_validation_endpoint,
        certificate_validation_callback,
    )?;

    let mut tls_stream = {
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
