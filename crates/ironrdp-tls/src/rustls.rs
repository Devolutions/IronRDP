use std::io;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

pub type TlsStream<S> = tokio_rustls::client::TlsStream<S>;

pub async fn upgrade<S>(stream: S, server_name: &str) -> io::Result<(TlsStream<S>, Vec<u8>)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let mut tls_stream = {
        let mut config = tokio_rustls::rustls::client::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(std::sync::Arc::new(danger::NoCertificateVerification))
            .with_no_client_auth();

        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        config.key_log = std::sync::Arc::new(tokio_rustls::rustls::KeyLogFile::new());

        // Disable TLS resumption because it’s not supported by some services such as CredSSP.
        //
        // > The CredSSP Protocol does not extend the TLS wire protocol. TLS session resumption is not supported.
        //
        // source: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cssp/385a7489-d46b-464c-b224-f7340e308a5c
        config.resumption = tokio_rustls::rustls::client::Resumption::disabled();

        let config = std::sync::Arc::new(config);

        let server_name = server_name.try_into().unwrap();

        tokio_rustls::TlsConnector::from(config)
            .connect(server_name, stream)
            .await?
    };

    tls_stream.flush().await?;

    let server_public_key = {
        let cert = tls_stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certificates| certificates.first())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "peer certificate is missing"))?;
        crate::extract_tls_server_public_key(&cert.0)?
    };

    Ok((tls_stream, server_public_key))
}

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
