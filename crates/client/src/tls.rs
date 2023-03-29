use std::io;

use ironrdp::session::connection_sequence::UpgradedStream;
use ironrdp::session::RdpError;
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncReadCompatExt as _;
use x509_parser::prelude::{FromDer as _, X509Certificate};

#[cfg(feature = "rustls")]
type TlsStream = tokio_util::compat::Compat<tokio_rustls::client::TlsStream<TcpStream>>;

#[cfg(all(feature = "native-tls", not(feature = "rustls")))]
type TlsStream = tokio_util::compat::Compat<async_native_tls::TlsStream<TcpStream>>;

// TODO: this can be refactored into a separate `ironrdp-tls` crate (all native clients will do the same TLS dance)
pub async fn establish_tls(
    stream: tokio_util::compat::Compat<TcpStream>,
) -> Result<UpgradedStream<TlsStream>, RdpError> {
    let stream = stream.into_inner();

    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    let mut tls_stream = {
        let connector = async_native_tls::TlsConnector::new()
            .danger_accept_invalid_certs(true)
            .use_sni(false);

        // domain is an empty string because client accepts IP address in the cli
        match connector.connect("", stream).await {
            Ok(tls) => tls,
            Err(err) => return Err(RdpError::TlsHandshake(err)),
        }
    };

    #[cfg(feature = "rustls")]
    let mut tls_stream = {
        let mut client_config = tokio_rustls::rustls::client::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(std::sync::Arc::new(danger::NoCertificateVerification))
            .with_no_client_auth();
        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        client_config.key_log = std::sync::Arc::new(tokio_rustls::rustls::KeyLogFile::new());
        let rc_config = std::sync::Arc::new(client_config);
        let example_com = "stub_string".try_into().unwrap();
        let connector = tokio_rustls::TlsConnector::from(rc_config);
        connector.connect(example_com, stream).await?
    };

    tls_stream.flush().await?;

    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    let server_public_key = {
        let cert = tls_stream
            .peer_certificate()
            .map_err(RdpError::TlsConnector)?
            .ok_or(RdpError::MissingPeerCertificate)?;
        get_tls_peer_pubkey(cert.to_der().map_err(RdpError::DerEncode)?)?
    };

    #[cfg(feature = "rustls")]
    let server_public_key = {
        let cert = tls_stream
            .get_ref()
            .1
            .peer_certificates()
            .ok_or(RdpError::MissingPeerCertificate)?[0]
            .as_ref();
        get_tls_peer_pubkey(cert.to_vec())?
    };

    Ok(UpgradedStream {
        stream: tls_stream.compat(),
        server_public_key,
    })
}

fn get_tls_peer_pubkey(cert: Vec<u8>) -> io::Result<Vec<u8>> {
    let res = X509Certificate::from_der(&cert[..])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid der certificate."))?;
    let public_key = res.1.tbs_certificate.subject_pki.subject_public_key;

    Ok(public_key.data.to_vec())
}

#[cfg(feature = "rustls")]
mod danger {
    use std::time::SystemTime;

    use tokio_rustls::rustls::client::ServerCertVerified;
    use tokio_rustls::rustls::{Certificate, Error, ServerName};

    pub struct NoCertificateVerification;

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
