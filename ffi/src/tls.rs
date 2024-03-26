use std::{io::Write, net::TcpStream};

use anyhow::Context;

pub type TlsStream = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

#[diplomat::bridge]
pub mod ffi {
    use crate::{
        error::{ffi::IronRdpError, ValueConsumedError},
        utils::ffi::VecU8,
    };

    use super::TlsStream;

    #[diplomat::opaque]
    pub struct Tls;

    #[diplomat::opaque]
    pub struct TlsUpgradeResult {
        pub tls_stream: Option<TlsStream>,
        pub server_public_key: Vec<u8>,
    }

    #[diplomat::opaque]
    pub struct UpgradedStream(pub Option<TlsStream>);

    impl TlsUpgradeResult {
        pub fn get_upgraded_stream(&mut self) -> Result<Box<UpgradedStream>, Box<IronRdpError>> {
            let Some(tls_stream) = self.tls_stream.take() else {
                return Err(ValueConsumedError::for_item("tls_stream")
                    .reason("tls_stream is missing")
                    .into());
            };

            Ok(Box::new(UpgradedStream(Some(tls_stream))))
        }

        pub fn get_server_public_key(&self) -> Box<VecU8> {
            VecU8::from_byte(&self.server_public_key)
        }
    }

    impl Tls {
        pub fn tls_upgrade(
            stream: &mut crate::utils::ffi::StdTcpStream,
            server_name: &str,
        ) -> Result<Box<TlsUpgradeResult>, Box<IronRdpError>> {
            let Some(stream) = stream.0.take() else {
                return Err(ValueConsumedError::for_item("stream")
                    .reason("inner stream is missing")
                    .into());
            };
            let (tls_stream, server_public_key) = super::tls_upgrade(stream, server_name).unwrap();

            Ok(Box::new(TlsUpgradeResult {
                tls_stream: Some(tls_stream),
                server_public_key,
            }))
        }
    }
}

/// Copy and parsed this code from the screenshot example, this is temporary and should be replaced with something more configurable
/// Need to put more thought into this
/// FIXME/TODO, Implement better TLS FFI interface
fn tls_upgrade(
    stream: TcpStream,
    server_name: &str,
) -> anyhow::Result<(rustls::StreamOwned<rustls::ClientConnection, TcpStream>, Vec<u8>)> {
    let mut config = rustls::client::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(std::sync::Arc::new(danger::NoCertificateVerification))
        .with_no_client_auth();

    // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
    config.key_log = std::sync::Arc::new(rustls::KeyLogFile::new());

    // Disable TLS resumption because it’s not supported by some services such as CredSSP.
    //
    // > The CredSSP Protocol does not extend the TLS wire protocol. TLS session resumption is not supported.
    //
    // source: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-cssp/385a7489-d46b-464c-b224-f7340e308a5c
    config.resumption = rustls::client::Resumption::disabled();

    let config = std::sync::Arc::new(config);

    let server_name = server_name.try_into().unwrap();

    let client = rustls::ClientConnection::new(config, server_name)?;

    let mut tls_stream = rustls::StreamOwned::new(client, stream);

    // We need to flush in order to ensure the TLS handshake is moving forward. Without flushing,
    // it’s likely the peer certificate is not yet received a this point.
    tls_stream.flush()?;

    let cert = tls_stream
        .conn
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .context("peer certificate is missing")?;

    let server_public_key = extract_tls_server_public_key(&cert.0)?;

    Ok((tls_stream, server_public_key))
}

fn extract_tls_server_public_key(cert: &[u8]) -> anyhow::Result<Vec<u8>> {
    use x509_cert::der::Decode as _;

    let cert = x509_cert::Certificate::from_der(cert)?;

    let server_public_key = cert
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .context("subject public key BIT STRING is not aligned")?
        .to_owned();

    Ok(server_public_key)
}

mod danger {
    use std::time::SystemTime;

    use rustls::client::{ServerCertVerified, ServerCertVerifier};
    use rustls::{Certificate, Error, ServerName};

    pub(super) struct NoCertificateVerification;

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &Certificate,
            _intermediates: &[Certificate],
            _server_name: &ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            _ocsp_response: &[u8],
            _now: SystemTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }
    }
}
