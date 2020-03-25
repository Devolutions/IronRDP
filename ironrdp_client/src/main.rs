mod config;

use std::{
    io::{self, Write},
    net::TcpStream,
    sync::Arc,
};

use ironrdp::{nego, rdp};
use log::{debug, error, warn};
use sspi::internal::credssp;

use self::config::Config;
use ironrdp_client::{
    active_session::process_active_stage,
    connection_sequence::{
        process_capability_sets, process_cred_ssp, process_finalization, process_mcs,
        process_mcs_connect, process_server_license_exchange, send_client_info,
    },
    transport::{
        connect, EarlyUserAuthResult, McsTransport, SendDataContextTransport,
        ShareControlHeaderTransport, ShareDataHeaderTransport,
    },
    RdpError,
};

const BUF_READER_SIZE: usize = 32 * 1024;

mod danger {
    pub struct NoCertificateVerification {}

    impl rustls::ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _roots: &rustls::RootCertStore,
            _presented_certs: &[rustls::Certificate],
            _dns_name: webpki::DNSNameRef<'_>,
            _ocsp: &[u8],
        ) -> Result<rustls::ServerCertVerified, rustls::TLSError> {
            Ok(rustls::ServerCertVerified::assertion())
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
    let stream = TcpStream::connect(addr.as_str()).map_err(RdpError::ConnectionError)?;
    let mut stream = bufstream::BufStream::new(stream);

    let (mut transport, selected_protocol) = connect(
        &mut stream,
        config.input.security_protocol,
        config.input.credentials.username.clone(),
    )?;

    let mut stream = stream.into_inner().map_err(io::Error::from)?;

    let mut client_config = rustls::ClientConfig::default();
    client_config
        .dangerous()
        .set_certificate_verifier(Arc::new(danger::NoCertificateVerification {}));
    let config_ref = Arc::new(client_config);
    let dns_name = webpki::DNSNameRef::try_from_ascii_str("stub_string").unwrap();
    let mut tls_session = rustls::ClientSession::new(&config_ref, dns_name);

    let mut tls_stream = rustls::Stream::new(&mut tls_session, &mut stream);
    // handshake
    tls_stream.flush()?;

    let mut tls_stream =
        bufstream::BufStream::with_capacities(BUF_READER_SIZE, BUF_READER_SIZE, tls_stream);

    if selected_protocol.contains(nego::SecurityProtocol::HYBRID)
        || selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX)
    {
        process_cred_ssp(&mut tls_stream, config.input.credentials.clone())?;

        if selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX) {
            if let credssp::EarlyUserAuthResult::AccessDenied =
                EarlyUserAuthResult::read(&mut tls_stream)?
            {
                return Err(RdpError::AccessDenied);
            }
        }
    }

    let static_channels = process_mcs_connect(
        &mut tls_stream,
        &mut transport,
        &config.input,
        selected_protocol,
    )?;

    let mut transport = McsTransport::new(transport);
    let joined_static_channels = process_mcs(
        &mut tls_stream,
        &mut transport,
        static_channels,
        &config.input,
    )?;
    debug!("Joined static active_session: {:?}", joined_static_channels);

    let global_channel_id = *joined_static_channels
        .get(config.input.global_channel_name.as_str())
        .expect("global channel must be added");
    let initiator_id = *joined_static_channels
        .get(config.input.user_channel_name.as_str())
        .expect("user channel must be added");

    let mut transport = SendDataContextTransport::new(transport, initiator_id, global_channel_id);
    send_client_info(
        &mut tls_stream,
        &mut transport,
        &config.input,
        &config.routing_addr,
    )?;

    match process_server_license_exchange(
        &mut tls_stream,
        &mut transport,
        &config.input,
        global_channel_id,
    ) {
        Err(RdpError::ServerLicenseError(rdp::RdpError::ServerLicenseError(
            rdp::server_license::ServerLicenseError::UnexpectedValidClientError(_),
        ))) => {
            warn!("The server has returned STATUS_VALID_CLIENT unexpectedly");
        }
        Err(error) => return Err(error),
        Ok(_) => (),
    }

    let mut transport =
        ShareControlHeaderTransport::new(transport, initiator_id, global_channel_id);
    let desktop_sizes = process_capability_sets(&mut tls_stream, &mut transport, &config.input)?;

    let mut transport = ShareDataHeaderTransport::new(transport);
    process_finalization(&mut tls_stream, &mut transport, initiator_id)?;

    process_active_stage(
        &mut tls_stream,
        joined_static_channels,
        global_channel_id,
        initiator_id,
        desktop_sizes,
        config.input,
    )?;

    Ok(())
}

pub fn socket_addr_to_string(socket_addr: std::net::SocketAddr) -> String {
    format!("{}:{}", socket_addr.ip(), socket_addr.port())
}
