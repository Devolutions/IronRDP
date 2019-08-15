mod config;
mod connection_sequence;
mod utils;

use std::{io, net::TcpStream};

use failure::Fail;
use log::{debug, error};
use native_tls::TlsConnector;

use self::{config::Config, connection_sequence::*};

pub type RdpResult<T> = Result<T, RdpError>;

fn main() {
    let config = Config::parse_args();
    setup_logging(config.log_file.clone()).expect("failed to initialize logging");

    match run(config) {
        Ok(_) => {
            println!("RDP connection sequence finished");
            std::process::exit(exitcode::OK);
        }
        Err(e) => {
            error!("{}", e);
            println!("RDP failed because of {}", e);

            let code = match e {
                RdpError::IOError(_) => exitcode::IOERR,
                RdpError::ConnectionError(_) => exitcode::NOHOST,
                _ => exitcode::PROTOCOL,
            };
            std::process::exit(code);
        }
    }
}

fn setup_logging(log_file: String) -> Result<(), fern::InitError> {
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

fn run(config: Config) -> RdpResult<()> {
    let addr = utils::socket_addr_to_string(config.routing_addr);
    let mut stream = TcpStream::connect(addr.as_str()).map_err(RdpError::ConnectionError)?;

    let selected_protocol = process_negotiation(
        &mut stream,
        config.input.credentials.username.clone(),
        config.input.security_protocol,
        ironrdp::NegotiationRequestFlags::empty(),
    )?;

    let mut tls_stream = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()?
        .connect(addr.as_str(), stream)?;

    if selected_protocol.contains(ironrdp::SecurityProtocol::HYBRID)
        || selected_protocol.contains(ironrdp::SecurityProtocol::HYBRID_EX)
    {
        process_cred_ssp(&mut tls_stream, config.input.credentials.clone())?;

        if selected_protocol.contains(ironrdp::SecurityProtocol::HYBRID_EX) {
            if let sspi::EarlyUserAuthResult::AccessDenied =
                EarlyUserAuthResult::read(&mut tls_stream)?
            {
                return Err(RdpError::AccessDenied);
            }
        }
    }

    let static_channels = process_mcs_connect(&mut tls_stream, &config, selected_protocol)?;

    let joined_static_channels = process_mcs(&mut tls_stream, static_channels)?;
    debug!("Joined static channels: {:?}", joined_static_channels);

    let global_channel_id = *joined_static_channels
        .get(&*GLOBAL_CHANNEL_NAME)
        .expect("global channel must be added");
    let initiator_id = *joined_static_channels
        .get(&*USER_CHANNEL_NAME)
        .expect("user channel must be added");

    let mut transport = SendDataContextTransport::new(initiator_id, global_channel_id);

    send_client_info(&mut transport, &mut tls_stream, &config)?;
    process_server_license(&mut transport, &mut tls_stream)?;

    let mut transport = ShareControlHeaderTransport::new(transport, initiator_id);
    process_capability_sets(&mut transport, &mut tls_stream, &config)?;

    let mut transport = ShareDataHeaderTransport(transport);
    process_finalization(&mut transport, &mut tls_stream, initiator_id)?;

    Ok(())
}

#[derive(Debug, Fail)]
pub enum RdpError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "connection error: {}", _0)]
    ConnectionError(#[fail(cause)] io::Error),
    #[fail(display = "X.224 error: {}", _0)]
    X224Error(#[fail(cause)] io::Error),
    #[fail(display = "negotiation error: {}", _0)]
    NegotiationError(#[fail(cause)] io::Error),
    #[fail(display = "unexpected PDU: {}", _0)]
    UnexpectedPdu(String),
    #[fail(display = "invalid response: {}", _0)]
    InvalidResponse(String),
    #[fail(display = "TLS connector error: {}", _0)]
    TlsConnectorError(#[fail(cause)] native_tls::Error),
    #[fail(display = "TLS handshake error: {}", _0)]
    TlsHandshakeError(#[fail(cause)] native_tls::HandshakeError<TcpStream>),
    #[fail(display = "CredSSP error: {}", _0)]
    CredSspError(#[fail(cause)] sspi::SspiError),
    #[fail(display = "CredSSP TSRequest error: {}", _0)]
    TsRequestError(#[fail(cause)] io::Error),
    #[fail(display = "early User Authentication Result error: {}", _0)]
    EarlyUserAuthResultError(#[fail(cause)] io::Error),
    #[fail(display = "the server denied access via Early User Authentication Result")]
    AccessDenied,
    #[fail(display = "MCS Connect error: {}", _0)]
    McsConnectError(#[fail(cause)] ironrdp::McsError),
    #[fail(display = "failed to get info about the user: {}", _0)]
    UserInfoError(String),
    #[fail(display = "MCS error: {}", _0)]
    McsError(ironrdp::McsError),
    #[fail(display = "Client Info PDU error: {}", _0)]
    ClientInfoError(ironrdp::rdp::RdpError),
    #[fail(display = "Server License PDU error: {}", _0)]
    ServerLicenseError(ironrdp::rdp::RdpError),
    #[fail(display = "Share Control Header error: {}", _0)]
    ShareControlHeaderError(ironrdp::rdp::RdpError),
    #[fail(display = "capability sets error: {}", _0)]
    CapabilitySetsError(ironrdp::rdp::RdpError),
}

impl From<io::Error> for RdpError {
    fn from(e: io::Error) -> Self {
        RdpError::IOError(e)
    }
}

impl From<native_tls::Error> for RdpError {
    fn from(e: native_tls::Error) -> Self {
        RdpError::TlsConnectorError(e)
    }
}

impl From<native_tls::HandshakeError<TcpStream>> for RdpError {
    fn from(e: native_tls::HandshakeError<TcpStream>) -> Self {
        RdpError::TlsHandshakeError(e)
    }
}
