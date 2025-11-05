//! Example of utilizing IronRDP in a blocking, synchronous fashion.
//!
//! This example showcases the use of IronRDP in a blocking manner. It
//! demonstrates how to create a basic RDP client with just a few hundred lines
//! of code by leveraging the IronRDP crates suite.
//!
//! In this basic client implementation, the client establishes a connection
//! with the destination server, decodes incoming graphics updates, and saves the
//! resulting output as a PNG image file on the disk.
//!
//! # Usage example
//!
//! ```shell
//! cargo run --example=screenshot -- --host <HOSTNAME> -u <USERNAME> -p <PASSWORD> -o out.png
//! ```

#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary
#![allow(clippy::print_stdout)]

use core::time::Duration;
use std::io::Write as _;
use std::net::TcpStream;
use std::path::PathBuf;

use anyhow::Context as _;
use connector::Credentials;
use ironrdp::connector;
use ironrdp::connector::ConnectionResult;
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use sspi::network_client::reqwest_network_client::ReqwestNetworkClient;
use tokio_rustls::rustls;
use tracing::{debug, info, trace};

const HELP: &str = "\
USAGE:
  cargo run --example=screenshot -- --host <HOSTNAME> --port <PORT>
                                    -u/--username <USERNAME> -p/--password <PASSWORD>
                                    [-o/--output <OUTPUT_FILE>] [-d/--domain <DOMAIN>]
";

fn main() -> anyhow::Result<()> {
    let action = match parse_args() {
        Ok(action) => action,
        Err(e) => {
            println!("{HELP}");
            return Err(e.context("invalid argument(s)"));
        }
    };

    setup_logging()?;

    match action {
        Action::ShowHelp => {
            println!("{HELP}");
            Ok(())
        }
        Action::Run {
            host,
            port,
            username,
            password,
            output,
            domain,
        } => {
            info!(host, port, username, password, output = %output.display(), domain, "run");
            run(host, port, username, password, output, domain)
        }
    }
}

#[derive(Debug)]
enum Action {
    ShowHelp,
    Run {
        host: String,
        port: u16,
        username: String,
        password: String,
        output: PathBuf,
        domain: Option<String>,
    },
}

fn parse_args() -> anyhow::Result<Action> {
    let mut args = pico_args::Arguments::from_env();

    let action = if args.contains(["-h", "--help"]) {
        Action::ShowHelp
    } else {
        let host = args.value_from_str("--host")?;
        let port = args.opt_value_from_str("--port")?.unwrap_or(3389);
        let username = args.value_from_str(["-u", "--username"])?;
        let password = args.value_from_str(["-p", "--password"])?;
        let output = args
            .opt_value_from_str(["-o", "--output"])?
            .unwrap_or_else(|| PathBuf::from("out.png"));
        let domain = args.opt_value_from_str(["-d", "--domain"])?;

        Action::Run {
            host,
            port,
            username,
            password,
            output,
            domain,
        }
    };

    Ok(action)
}

fn setup_logging() -> anyhow::Result<()> {
    use tracing::metadata::LevelFilter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let fmt_layer = tracing_subscriber::fmt::layer().compact();

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .with_env_var("IRONRDP_LOG")
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(env_filter)
        .try_init()
        .context("failed to set tracing global subscriber")?;

    Ok(())
}

fn run(
    server_name: String,
    port: u16,
    username: String,
    password: String,
    output: PathBuf,
    domain: Option<String>,
) -> anyhow::Result<()> {
    let config = build_config(username, password, domain);

    let (connection_result, framed) = connect(config, server_name, port).context("connect")?;

    let mut image = DecodedImage::new(
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    active_stage(connection_result, framed, &mut image).context("active stage")?;

    let img: image::ImageBuffer<image::Rgba<u8>, _> =
        image::ImageBuffer::from_raw(u32::from(image.width()), u32::from(image.height()), image.data())
            .context("invalid image")?;

    img.save(output).context("save image to disk")?;

    Ok(())
}

fn build_config(username: String, password: String, domain: Option<String>) -> connector::Config {
    connector::Config {
        credentials: Credentials::UsernamePassword { username, password },
        domain,
        // Prefer TLS without CredSSP to maximize compatibility with servers
        // that have NLA disabled but accept SSL/TLS security.
        enable_tls: true,
        enable_credssp: true,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize {
            width: 1280,
            height: 1024,
        },
        bitmap: None,
        client_build: 0,
        client_name: "ironrdp-screenshot-example".to_owned(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),

        #[cfg(windows)]
        platform: MajorPlatformType::WINDOWS,
        #[cfg(target_os = "macos")]
        platform: MajorPlatformType::MACINTOSH,
        #[cfg(target_os = "ios")]
        platform: MajorPlatformType::IOS,
        #[cfg(target_os = "linux")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "android")]
        platform: MajorPlatformType::ANDROID,
        #[cfg(target_os = "freebsd")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "dragonfly")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "openbsd")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "netbsd")]
        platform: MajorPlatformType::UNIX,

        enable_server_pointer: false, // Disable custom pointers (there is no user interaction anyway).
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        pointer_software_rendering: true,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
    }
}

type UpgradedFramed = ironrdp_blocking::Framed<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>;

fn connect(
    config: connector::Config,
    server_name: String,
    port: u16,
) -> anyhow::Result<(ConnectionResult, UpgradedFramed)> {
    let server_addr = lookup_addr(&server_name, port).context("lookup addr")?;

    info!(%server_addr, "Looked up server address");

    let tcp_stream = TcpStream::connect(server_addr).context("TCP connect")?;

    // Sets the read timeout for the TCP stream so we can break out of the
    // infinite loop during the active stage once there is no more activity.
    tcp_stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("set_read_timeout call failed");

    let client_addr = tcp_stream.local_addr().context("get socket local address")?;

    let mut framed = ironrdp_blocking::Framed::new(tcp_stream);

    let mut connector = connector::ClientConnector::new(config, client_addr);

    let should_upgrade = ironrdp_blocking::connect_begin(&mut framed, &mut connector).context("begin connection")?;

    debug!("TLS upgrade");

    // Ensure there is no leftover
    let initial_stream = framed.into_inner_no_leftover();

    let (upgraded_stream, server_public_key) =
        tls_upgrade(initial_stream, server_name.clone()).context("TLS upgrade")?;

    let upgraded = ironrdp_blocking::mark_as_upgraded(should_upgrade, &mut connector);

    let mut upgraded_framed = ironrdp_blocking::Framed::new(upgraded_stream);

    let mut network_client = ReqwestNetworkClient;
    let connection_result = ironrdp_blocking::connect_finalize(
        upgraded,
        &mut upgraded_framed,
        connector,
        server_name.into(),
        server_public_key,
        &mut network_client,
        None,
    )
    .context("finalize connection")?;

    Ok((connection_result, upgraded_framed))
}

fn active_stage(
    connection_result: ConnectionResult,
    mut framed: UpgradedFramed,
    image: &mut DecodedImage,
) -> anyhow::Result<()> {
    let mut active_stage = ActiveStage::new(connection_result);

    'outer: loop {
        let (action, payload) = match framed.read_pdu() {
            Ok((action, payload)) => (action, payload),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break 'outer,
            Err(e) => return Err(anyhow::Error::new(e).context("read frame")),
        };

        trace!(?action, frame_length = payload.len(), "Frame received");

        // Temporary introspection to pinpoint X.224 decode failures
        if matches!(action, ironrdp::pdu::Action::X224) {
            use ironrdp::connector::legacy as legacy_conn;
            match legacy_conn::decode_send_data_indication(&payload) {
                Ok(ctx) => {
                    let dump_len = ctx.user_data.len().min(32);
                    let mut first_bytes = String::new();
                    for b in &ctx.user_data[..dump_len] {
                        use core::fmt::Write as _;
                        let _ = write!(&mut first_bytes, "{:02x} ", b);
                    }
                    tracing::debug!(
                        channel_id = ctx.channel_id,
                        initiator_id = ctx.initiator_id,
                        user_data_len = ctx.user_data.len(),
                        first_bytes = %first_bytes,
                        "X224 SendDataIndication"
                    );
                    // Try to decode share control header to see PDU type
                    match legacy_conn::decode_share_control(ctx) {
                        Ok(sc) => {
                            tracing::debug!(
                                pdu_type = %sc.pdu.as_short_name(),
                                "X224 payload contains ShareControlPDU"
                            );
                            if let ironrdp::pdu::rdp::headers::ShareControlPdu::Data(share_data) = sc.pdu {
                                tracing::debug!(
                                    share_data_pdu_type = %share_data.share_data_pdu.as_short_name(),
                                    "ShareData PDU"
                                );
                            }
                        }
                        Err(e) => {
                            // Using e.report() for detailed errors.
                            tracing::warn!("Failed to decode ShareControl header: {}", e.report());
                            // Perform a light-weight peek of ShareControl/ShareData header fixed fields
                            if let Some(info) = peek_share_data_header_fixed(ctx.user_data) {
                                tracing::warn!(
                                    total_len = info.total_len,
                                    sc_pdu_type = info.sc_pdu_type,
                                    stream_id = info.stream_id,
                                    uncompressed_len = info.uncompressed_len,
                                    sd_pdu_type = info.sd_pdu_type,
                                    compression_flags_raw = info.compression_flags_raw,
                                    compression_type_raw = info.compression_type_raw,
                                    compressed_len = info.compressed_len,
                                    update_type = info.update_type,
                                    nrect = info.nrect,
                                    first_rect_bpp = info.first_rect_bpp,
                                    first_rect_flags = info.first_rect_flags,
                                    first_rect_len = info.first_rect_len,
                                    "Peeked ShareDataHeader fields"
                                );
                                // Dump first 32 bytes of update payload (after updateType+pad)
                                let base = 10usize; // ShareControlHeader fixed size
                                let upd_off = base + 8; // ShareDataHeader fixed size
                                let payload_off = upd_off + 4; // updateType (2) + pad2 (2)
                                let end = core::cmp::min(payload_off + 32, ctx.user_data.len());
                                let mut buf = String::new();
                                for b in &ctx.user_data[payload_off..end] {
                                    use core::fmt::Write as _;
                                    let _ = write!(&mut buf, "{:02x} ", b);
                                }
                                tracing::warn!(first_payload_bytes = %buf, "Update payload head");
                            } else {
                                tracing::warn!("Peek failed to parse ShareDataHeader fixed fields");
                            }
                        }
                    }
                }
                Err(e) => {
                    // Using e.report() for detailed errors.
                    tracing::warn!("Failed to decode SendDataIndication: {}", e.report());
                }
            }
        }

        let outputs = active_stage.process(image, action, &payload)?;

        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => framed.write_all(&frame).context("write response")?,
                ActiveStageOutput::Terminate(_) => break 'outer,
                _ => {}
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ShareDataPeekInfo {
    total_len: u16,
    sc_pdu_type: u16,
    stream_id: u8,
    uncompressed_len: u16,
    sd_pdu_type: u8,
    compression_flags_raw: u8,
    compression_type_raw: u8,
    compressed_len: u16,
    update_type: Option<u16>,
    nrect: Option<u16>,
    first_rect_bpp: Option<u16>,
    first_rect_flags: Option<u16>,
    first_rect_len: Option<u16>,
}

fn peek_share_data_header_fixed(buf: &[u8]) -> Option<ShareDataPeekInfo> {
    // ShareControlHeader fixed: 2 (len) + 2 (type+ver) + 2 (pdu_source) + 4 (share_id)
    if buf.len() < 10 {
        return None;
    }
    let total_len = u16::from_le_bytes([buf[0], buf[1]]);
    let sc_pdu_type = u16::from_le_bytes([buf[2], buf[3]]);
    // let _pdu_source = u16::from_le_bytes([buf[4], buf[5]]);
    // let _share_id = u32::from_le_bytes([buf[6], buf[7], buf[8], buf[9]]);

    // Data PDU type is 0x7 (masked by 0xF)
    if (sc_pdu_type & 0x000F) != 0x0007 {
        return None;
    }

    if buf.len() < 10 + 1 + 1 + 2 + 1 + 1 + 2 {
        return None;
    }
    let base = 10;
    let _pad = buf[base];
    let stream_id = buf[base + 1];
    let uncompressed_len = u16::from_le_bytes([buf[base + 2], buf[base + 3]]);
    let sd_pdu_type = buf[base + 4];
    let comp_flags_with_type = buf[base + 5];
    let compressed_len = u16::from_le_bytes([buf[base + 6], buf[base + 7]]);

    let mut info = ShareDataPeekInfo {
        total_len,
        sc_pdu_type,
        stream_id,
        uncompressed_len,
        sd_pdu_type,
        compression_flags_raw: comp_flags_with_type & !ironrdp::pdu::rdp::headers::SHARE_DATA_HEADER_COMPRESSION_MASK,
        compression_type_raw: comp_flags_with_type & ironrdp::pdu::rdp::headers::SHARE_DATA_HEADER_COMPRESSION_MASK,
        compressed_len,
        update_type: None,
        nrect: None,
        first_rect_bpp: None,
        first_rect_flags: None,
        first_rect_len: None,
    };

    // Try to read updateType (u16) after ShareDataHeader fixed part
    let upd_off = base + 8;
    if buf.len() >= upd_off + 4 {
        let upd_type = u16::from_le_bytes([buf[upd_off], buf[upd_off + 1]]);
        info.update_type = Some(upd_type);
        // TS_UPDATE_BITMAP_DATA starts at upd_off + 4
        if upd_type == 1 && buf.len() >= upd_off + 6 {
            let nrect = u16::from_le_bytes([buf[upd_off + 4], buf[upd_off + 5]]);
            info.nrect = Some(nrect);
            // Try to parse first TS_BITMAP_DATA minimal header fields
            let rect_off = upd_off + 6;
            if buf.len() >= rect_off + 8 + 2 + 2 + 2 + 2 + 2 {
                // skip rectangle (8 bytes)
                let bpp = u16::from_le_bytes([buf[rect_off + 8 + 4], buf[rect_off + 8 + 5]]);
                let flags = u16::from_le_bytes([buf[rect_off + 8 + 6], buf[rect_off + 8 + 7]]);
                let len = u16::from_le_bytes([buf[rect_off + 8 + 8], buf[rect_off + 8 + 9]]);
                info.first_rect_bpp = Some(bpp);
                info.first_rect_flags = Some(flags);
                info.first_rect_len = Some(len);
            }
        }
    }

    Some(info)
}

fn lookup_addr(hostname: &str, port: u16) -> anyhow::Result<core::net::SocketAddr> {
    use std::net::ToSocketAddrs as _;
    let addr = (hostname, port).to_socket_addrs()?.next().unwrap();
    Ok(addr)
}

fn tls_upgrade(
    stream: TcpStream,
    server_name: String,
) -> anyhow::Result<(rustls::StreamOwned<rustls::ClientConnection, TcpStream>, Vec<u8>)> {
    let mut config = rustls::client::ClientConfig::builder()
        .dangerous()
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

    let server_public_key = extract_tls_server_public_key(cert)?;

    Ok((tls_stream, server_public_key))
}

fn extract_tls_server_public_key(cert: &[u8]) -> anyhow::Result<Vec<u8>> {
    use x509_cert::der::Decode as _;

    let cert = x509_cert::Certificate::from_der(cert)?;

    debug!(%cert.tbs_certificate.subject);

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
    use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use tokio_rustls::rustls::{pki_types, DigitallySignedStruct, Error, SignatureScheme};

    #[derive(Debug)]
    pub(super) struct NoCertificateVerification;

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _: &pki_types::CertificateDer<'_>,
            _: &[pki_types::CertificateDer<'_>],
            _: &pki_types::ServerName<'_>,
            _: &[u8],
            _: pki_types::UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA1,
                SignatureScheme::ECDSA_SHA1_Legacy,
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP521_SHA512,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
                SignatureScheme::ED448,
            ]
        }
    }
}
