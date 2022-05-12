mod config;

use std::{
    io::{self, Write},
    net::TcpStream,
};

use ironrdp_client::{process_active_stage, process_connection_sequence, RdpError, UpgradedStream};
use log::error;
#[cfg(all(feature = "native-tls", not(feature = "rustls")))]
use native_tls::{HandshakeError, TlsConnector};

use self::config::Config;

#[cfg(feature = "rustls")]
mod danger {
    use rustls::client::ServerCertVerified;
    use rustls::{Certificate, Error, ServerName};
    use std::time::SystemTime;

    pub struct NoCertificateVerification;

    impl rustls::client::ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &Certificate,
            _intermediates: &[Certificate],
            _server_name: &ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            _ocsp_response: &[u8],
            _now: SystemTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(rustls::client::ServerCertVerified::assertion())
        }
    }
}

fn main() {
    let config = Config::parse_args();
    setup_logging(config.log_file.as_str()).expect("failed to initialize logging");

    let exit_code = match run(config) {
        Ok(_) => {
            println!("RDP successfully finished");

            exitcode::OK
        }
        Err(RdpError::IOError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
            error!("{}", e);
            println!("The server has terminated the RDP session");

            exitcode::NOHOST
        }
        Err(ref e) => {
            error!("{}", e);
            println!("RDP failed because of {}", e);

            match e {
                RdpError::IOError(_) => exitcode::IOERR,
                RdpError::ConnectionError(_) => exitcode::NOHOST,
                _ => exitcode::PROTOCOL,
            }
        }
    };

    std::process::exit(exit_code);
}

fn setup_logging(log_file: &str) -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S:%6f]"),
                record.level(),
                message
            ))
        })
        .chain(fern::log_file(log_file)?)
        .apply()?;

    Ok(())
}

fn run(config: Config) -> Result<(), RdpError> {
    let addr = socket_addr_to_string(config.routing_addr);
    let mut stream = TcpStream::connect(addr.as_str()).map_err(RdpError::ConnectionError)?;

    let (connection_sequence_result, mut stream) =
        process_connection_sequence(&mut stream, &config.routing_addr, &config.input, establish_tls)?;

    process_active_stage(&mut stream, config.input, connection_sequence_result)?;

    Ok(())
}

fn establish_tls(stream: impl io::Read + io::Write) -> Result<UpgradedStream<impl io::Read + io::Write>, RdpError> {
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    let mut tls_stream = {
        let mut builder = TlsConnector::builder();
        builder.danger_accept_invalid_certs(true);
        builder.use_sni(false);

        let connector = builder.build().unwrap();

        // domain is an empty string because client accepts IP address in the cli
        match connector.connect("", stream) {
            Ok(tls) => tls,
            Err(HandshakeError::Failure(err)) => return Err(RdpError::TlsHandshakeError(err)),
            Err(HandshakeError::WouldBlock(mut mid_stream)) => loop {
                match mid_stream.handshake() {
                    Ok(tls) => break tls,
                    Err(HandshakeError::Failure(err)) => return Err(RdpError::TlsHandshakeError(err)),
                    Err(HandshakeError::WouldBlock(mid)) => mid_stream = mid,
                };
            },
        }
    };

    #[cfg(feature = "rustls")]
    let mut tls_stream = {
        let client_config = rustls::client::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(std::sync::Arc::new(danger::NoCertificateVerification))
            .with_no_client_auth();
        let rc_config = std::sync::Arc::new(client_config);
        let example_com = "stub_string".try_into().unwrap();
        let client = rustls::ClientConnection::new(rc_config, example_com)?;
        rustls::StreamOwned::new(client, stream)
    };

    tls_stream.flush()?;

    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    let server_public_key = {
        let cert = tls_stream
            .peer_certificate()
            .map_err(RdpError::TlsConnectorError)?
            .ok_or(RdpError::MissingPeerCertificate)?;
        get_tls_peer_pubkey(cert.to_der().map_err(RdpError::DerEncode)?)?
    };

    #[cfg(feature = "rustls")]
    let server_public_key = {
        let cert = tls_stream
            .conn
            .peer_certificates()
            .ok_or(RdpError::MissingPeerCertificate)?[0]
            .as_ref();
        get_tls_peer_pubkey(cert.to_vec())?
    };

    Ok(UpgradedStream {
        stream: tls_stream,
        server_public_key,
    })
}

pub fn socket_addr_to_string(socket_addr: std::net::SocketAddr) -> String {
    format!("{}:{}", socket_addr.ip(), socket_addr.port())
}

pub fn get_tls_peer_pubkey(cert: Vec<u8>) -> io::Result<Vec<u8>> {
    let res = x509_parser::parse_x509_der(&cert[..])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid der certificate."))?;
    let public_key = res.1.tbs_certificate.subject_pki.subject_public_key;

    Ok(public_key.data.to_vec())
}
