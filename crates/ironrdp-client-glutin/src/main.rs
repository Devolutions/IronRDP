mod config;

use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::{io, process};

use anyhow::Context as _;
use futures_util::io::AsyncWriteExt as _;
use gui::MessagePassingGfxHandler;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::dvc::gfx::ServerPdu;
use ironrdp::session::connection_sequence::{process_connection_sequence, UpgradedStream};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageOutput, ActiveStageProcessor, ErasedWriter, RdpError};
use sspi::network_client::reqwest_network_client::RequestClientFactory;
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncReadCompatExt as _;
use x509_parser::prelude::{FromDer as _, X509Certificate};

use crate::config::Config;

#[cfg(feature = "rustls")]
type TlsStream = tokio_util::compat::Compat<tokio_rustls::client::TlsStream<TcpStream>>;

#[cfg(all(feature = "native-tls", not(feature = "rustls")))]
type TlsStream = tokio_util::compat::Compat<async_native_tls::TlsStream<TcpStream>>;

mod gui;

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

#[tokio::main]
async fn main() {
    let config = Config::parse_args();
    setup_logging(config.log_file.as_str()).expect("failed to initialize logging");

    let exit_code = match run(config).await {
        Ok(_) => {
            println!("RDP successfully finished");
            exitcode::OK
        }
        Err(RdpError::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
            error!("{}", e);
            println!("The server has terminated the RDP session");
            exitcode::NOHOST
        }
        Err(ref e) => {
            error!("{}", e);
            println!("RDP failed because of {e}");

            match e {
                RdpError::Io(_) => exitcode::IOERR,
                RdpError::Connection(_) => exitcode::NOHOST,
                _ => exitcode::PROTOCOL,
            }
        }
    };

    std::process::exit(exit_code);
}

fn setup_logging(log_file: &str) -> anyhow::Result<()> {
    use std::fs::OpenOptions;

    use tracing::metadata::LevelFilter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
        .with_context(|| format!("Couldn’t open {log_file}"))?;

    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(false)
        .with_writer(file);

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .with_env_var("IRONRDP_LOG_LEVEL")
        .from_env_lossy();

    let reg = tracing_subscriber::registry().with(fmt_layer).with(env_filter);

    tracing::subscriber::set_global_default(reg).context("Failed to set tracing global subscriber")?;

    Ok(())
}

async fn run(config: Config) -> Result<(), RdpError> {
    let addr = ironrdp::session::connection_sequence::Address::lookup_addr(&config.addr)?;

    let stream = TcpStream::connect(addr.sock).await.map_err(RdpError::Connection)?;

    let (connection_sequence_result, reader, writer) = process_connection_sequence(
        stream.compat(),
        &addr,
        &config.input,
        establish_tls,
        Box::new(RequestClientFactory),
    )
    .await?;

    let writer = Arc::new(Mutex::new(writer));
    let image = DecodedImage::new(
        PixelFormat::RgbA32,
        connection_sequence_result.desktop_size.width,
        connection_sequence_result.desktop_size.height,
    );

    launch_client(config, connection_sequence_result, image, reader, writer).await
}

async fn launch_client(
    config: Config,
    connection_sequence_result: ironrdp::session::connection_sequence::ConnectionSequenceResult,
    image: DecodedImage,
    reader: ironrdp::session::FramedReader,
    writer: Arc<Mutex<ErasedWriter>>,
) -> Result<(), RdpError> {
    let (sender, receiver) = sync_channel::<ServerPdu>(1);
    let handler = MessagePassingGfxHandler::new(sender);
    let active_stage = ActiveStageProcessor::new(
        config.input.clone(),
        Some(Box::new(handler)),
        connection_sequence_result,
    );
    let gui = gui::UiContext::new(config.input.width, config.input.height);

    let active_stage_writer = writer.clone();
    let active_stage_handle = tokio::spawn(async move {
        match process_active_stage(reader, active_stage, image, active_stage_writer).await {
            Ok(()) => Ok(()),
            Err(error) => {
                error!(?error, "Active stage failed");
                process::exit(-1);
            }
        }
    });
    gui::launch_gui(gui, config.gfx_dump_file, config.openh264_path, receiver, writer.clone())?;
    active_stage_handle.await.map_err(|e| RdpError::Io(e.into()))?
}

async fn process_active_stage(
    mut reader: ironrdp::session::FramedReader,
    mut active_stage: ActiveStageProcessor,
    mut image: DecodedImage,
    writer: Arc<Mutex<ErasedWriter>>,
) -> Result<(), RdpError> {
    'outer: loop {
        let frame = reader.read_frame().await?.ok_or(RdpError::AccessDenied)?.freeze();
        let outputs = active_stage.process(&mut image, frame)?;
        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => {
                    let mut writer = writer.lock().await;
                    writer.write_all(&frame).await?
                }
                ActiveStageOutput::GraphicsUpdate(_region) => {}
                ActiveStageOutput::Terminate => break 'outer,
            }
        }
    }
    Ok(())
}

// TODO: this can be refactored into a separate `ironrdp-tls` crate (all native clients will do the same TLS dance)
async fn establish_tls(stream: tokio_util::compat::Compat<TcpStream>) -> Result<UpgradedStream<TlsStream>, RdpError> {
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

pub fn get_tls_peer_pubkey(cert: Vec<u8>) -> io::Result<Vec<u8>> {
    let res = X509Certificate::from_der(&cert[..])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid der certificate."))?;
    let public_key = res.1.tbs_certificate.subject_pki.subject_public_key;

    Ok(public_key.data.to_vec())
}
