use std::io;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

#[cfg(feature = "rustls")]
pub type TlsStream<S> = tokio_rustls::client::TlsStream<S>;

#[cfg(all(feature = "native-tls", not(feature = "rustls")))]
pub type TlsStream<S> = tokio_native_tls::TlsStream<S>;

pub async fn upgrade<S>(stream: S, server_name: &str) -> io::Result<(TlsStream<S>, Vec<u8>)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    #[cfg(feature = "rustls")]
    let mut tls_stream = {
        // FIXME: disable TLS session resume just to be safe (not unsupported by CredSSP server)
        // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cssp/385a7489-d46b-464c-b224-f7340e308a5c
        // Option is available starting rustls 0.21

        let mut config = tokio_rustls::rustls::client::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(std::sync::Arc::new(danger::NoCertificateVerification))
            .with_no_client_auth();

        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        config.key_log = std::sync::Arc::new(tokio_rustls::rustls::KeyLogFile::new());

        let config = std::sync::Arc::new(config);

        let server_name = server_name.try_into().unwrap();

        tokio_rustls::TlsConnector::from(config)
            .connect(server_name, stream)
            .await?
    };

    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
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

    #[cfg(feature = "rustls")]
    let server_public_key = {
        let cert = tls_stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certificates| certificates.first())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "peer certificate is missing"))?;
        extract_tls_server_public_key(&cert.0)?
    };

    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    let server_public_key = {
        let cert = tls_stream
            .get_ref()
            .peer_certificate()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "peer certificate is missing"))?;
        let cert = cert.to_der().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        extract_tls_server_public_key(&cert)?
    };

    Ok((tls_stream, server_public_key))
}

fn extract_tls_server_public_key(cert: &[u8]) -> io::Result<Vec<u8>> {
    use x509_cert::der::Decode as _;

    let cert = x509_cert::Certificate::from_der(cert).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let server_public_key = cert
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .raw_bytes()
        .to_owned();

    Ok(server_public_key)
}

#[cfg(feature = "rustls")]
mod danger {
    use std::time::SystemTime;

    use tokio_rustls::rustls::client::ServerCertVerified;
    use tokio_rustls::rustls::{Certificate, Error, ServerName};

    pub(super) struct NoCertificateVerification;

    impl tokio_rustls::rustls::client::ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &Certificate,
            _intermediates: &[Certificate],
            _server_name: &ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            _ocsp_response: &[u8],
            _now: SystemTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(tokio_rustls::rustls::client::ServerCertVerified::assertion())
        }
    }
}
