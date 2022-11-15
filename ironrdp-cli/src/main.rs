#[macro_use]
extern crate log;

mod config;

use std::io;

use crate::config::Config;
use futures_util::io::AsyncWriteExt as _;
use ironrdp::codecs::rfx::image_processing::PixelFormat;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{process_connection_sequence, ActiveStageOutput, ActiveStageProcessor, RdpError, UpgradedStream};
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncReadCompatExt as _;
use x509_parser::prelude::{FromDer as _, X509Certificate};

#[cfg(feature = "rustls")]
type TlsStream = tokio_util::compat::Compat<tokio_rustls::client::TlsStream<TcpStream>>;

#[cfg(all(feature = "native-tls", not(feature = "rustls")))]
type TlsStream = tokio_util::compat::Compat<async_native_tls::TlsStream<TcpStream>>;

#[cfg(feature = "rustls")]
mod danger {
    use std::time::SystemTime;

    use rustls::client::ServerCertVerified;
    use rustls::{Certificate, Error, ServerName};

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

#[tokio::main]
async fn main() {
    let config = Config::parse_args();
    setup_logging(config.log_file.as_str()).expect("failed to initialize logging");

    let exit_code = match run(config).await {
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

async fn run(config: Config) -> Result<(), RdpError> {
    let stream = TcpStream::connect(config.routing_addr)
        .await
        .map_err(RdpError::ConnectionError)?;

    let (connection_sequence_result, mut reader, mut writer) =
        process_connection_sequence(stream.compat(), &config.routing_addr, &config.input, establish_tls).await?;

    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        u32::from(connection_sequence_result.desktop_size.width),
        u32::from(connection_sequence_result.desktop_size.height),
    );

    let mut active_stage = ActiveStageProcessor::new(config.input, connection_sequence_result);
    let mut frame_id = 0;

    'outer: loop {
        let frame = reader.read_frame().await?.ok_or(RdpError::AccessDenied)?;
        let outputs = active_stage.process(&mut image, frame).await?;
        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => writer.write_all(&frame).await?,
                ActiveStageOutput::GraphicsUpdate(_region) => {
                    // TODO: control this with CLI argument
                    dump_image(&image, frame_id);

                    frame_id += 1;
                }
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
            Err(err) => return Err(RdpError::TlsHandshakeError(err)),
        }
    };

    #[cfg(feature = "rustls")]
    let mut tls_stream = {
        let mut client_config = rustls::client::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(std::sync::Arc::new(danger::NoCertificateVerification))
            .with_no_client_auth();
        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        client_config.key_log = std::sync::Arc::new(rustls::KeyLogFile::new());
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
            .map_err(RdpError::TlsConnectorError)?
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

pub fn dump_image(image: &DecodedImage, frame_id: usize) {
    debug_assert_eq!(image.pixel_format(), PixelFormat::RgbA32);

    image::RgbaImage::from_raw(image.width(), image.height(), image.data().to_vec())
        .unwrap()
        .save(format!("frame.{frame_id}.jpg"))
        .unwrap();
}
