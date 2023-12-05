use std::io;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

pub type TlsStream<S> = tokio_native_tls::TlsStream<S>;

pub async fn upgrade<S>(stream: S, server_name: &str) -> io::Result<(TlsStream<S>, Vec<u8>)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let mut tls_stream = {
        let connector = tokio_native_tls::native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .use_sni(false)
            .build()
            .map(tokio_native_tls::TlsConnector::from)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        connector
            .connect(server_name, stream)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
    };

    tls_stream.flush().await?;

    let server_public_key = {
        let cert = tls_stream
            .get_ref()
            .peer_certificate()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "peer certificate is missing"))?;
        let cert = cert.to_der().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        crate::extract_tls_server_public_key(&cert)?
    };

    Ok((tls_stream, server_public_key))
}
