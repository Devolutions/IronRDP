mod config;

use std::{
    io::{self, Write},
    net::TcpStream,
};

use ironrdp_client::{process_active_stage, process_connection_sequence, RdpError, UpgradedStream};
use log::error;
use native_tls::TlsConnector;

use self::config::Config;

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
        .chain(std::io::stdout())
        .level(log::LevelFilter::Debug)
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

fn establish_tls(
    stream: impl io::Read + io::Write,
) -> Result<UpgradedStream<impl io::Read + io::Write>, RdpError> {
    let mut builder = TlsConnector::builder();
    builder.danger_accept_invalid_certs(true);
    builder.use_sni(false);

    let connector = builder.build().unwrap();

    let mut tls_stream = match connector.connect("", stream) {
        Ok(s) => s,
        Err(_) => panic!("on tls connect"),
    };

    // handshake
    tls_stream.flush()?;

    let cert = tls_stream
        .peer_certificate()
        .map_err(|err| RdpError::TlsConnectorError(err))?
        .ok_or(RdpError::MissingPeerCertificate)?;
    let server_public_key = get_tls_peer_pubkey(cert.to_der().unwrap())?;

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
