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
use ironrdp::pdu::gcc::{ConnectionType, KeyboardType};
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageBuilder, ActiveStageOutput};
use ironrdp_pdu::rdp::client_info::{CompressionType, PerformanceFlags, TimezoneInfo};
use sspi::network_client::reqwest_network_client::ReqwestNetworkClient;
use tokio_rustls::rustls;
use tracing::{debug, info, trace, warn};

const HELP: &str = "\
USAGE:
  cargo run --example=screenshot -- --host <HOSTNAME> -u/--username <USERNAME> -p/--password <PASSWORD>
                                    [--port <PORT>] [-o/--output <OUTPUT_FILE>] [-d/--domain <DOMAIN>]
                                    [--autologon <true|false>] [--tls-enabled <true|false>]
                                    [--credssp-enabled <true|false>]
                                    [--compression-enabled <true|false>] [--compression-level <0..3>]
                                    [--no-graphics-timeout-seconds <SECONDS>]
                                    [--after-first-graphics-seconds <SECONDS>]

WARNING:
  This example intentionally skips server certificate verification; use only with trusted servers.

OPTIONS:
  --port                         default: 3389
  --output                       default: out.png
  --autologon                    default: false
  --tls-enabled                  default: false (legacy TLS security protocol)
  --credssp-enabled              default: true (NLA)
  --compression-enabled          default: true
  --compression-level            default: 3
  --no-graphics-timeout-seconds  wait for first update; default: 3, greater than zero
  --after-first-graphics-seconds grace period after first update; default: 8, may be zero
";

fn main() -> anyhow::Result<()> {
    let action = match parse_args(pico_args::Arguments::from_env()) {
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
            autologon,
            tls_enabled,
            credssp_enabled,
            compression_enabled,
            compression_level,
            no_graphics_timeout_seconds,
            after_first_graphics_seconds,
        } => {
            info!(
                host,
                port,
                username,
                output = %output.display(),
                domain,
                autologon,
                tls_enabled,
                credssp_enabled,
                compression_enabled,
                compression_level,
                no_graphics_timeout_seconds,
                after_first_graphics_seconds,
                "run"
            );
            run(RunConfig {
                host,
                port,
                username,
                password,
                output,
                domain,
                autologon,
                tls_enabled,
                credssp_enabled,
                compression_enabled,
                compression_level,
                no_graphics_timeout_seconds,
                after_first_graphics_seconds,
            })
        }
    }
}

#[derive(Debug)]
struct RunConfig {
    host: String,
    port: u16,
    username: String,
    password: String,
    output: PathBuf,
    domain: Option<String>,
    autologon: bool,
    tls_enabled: bool,
    credssp_enabled: bool,
    compression_enabled: bool,
    compression_level: u32,
    no_graphics_timeout_seconds: u64,
    after_first_graphics_seconds: u64,
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
        autologon: bool,
        tls_enabled: bool,
        credssp_enabled: bool,
        compression_enabled: bool,
        compression_level: u32,
        no_graphics_timeout_seconds: u64,
        after_first_graphics_seconds: u64,
    },
}

fn parse_args(mut args: pico_args::Arguments) -> anyhow::Result<Action> {
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
        let autologon = args.opt_value_from_str("--autologon")?.unwrap_or(false);
        let tls_enabled = args.opt_value_from_str("--tls-enabled")?.unwrap_or(false);
        let credssp_enabled = args.opt_value_from_str("--credssp-enabled")?.unwrap_or(true);
        let compression_enabled = args.opt_value_from_str("--compression-enabled")?.unwrap_or(true);
        let compression_level = args.opt_value_from_str("--compression-level")?.unwrap_or(3);
        let no_graphics_timeout_seconds = args.opt_value_from_str("--no-graphics-timeout-seconds")?.unwrap_or(3);
        let after_first_graphics_seconds = args.opt_value_from_str("--after-first-graphics-seconds")?.unwrap_or(8);

        if compression_level > 3 {
            anyhow::bail!("Invalid compression level. Valid values are 0, 1, 2, 3.");
        }
        if no_graphics_timeout_seconds == 0 {
            anyhow::bail!("no-graphics-timeout-seconds must be greater than zero");
        }

        Action::Run {
            host,
            port,
            username,
            password,
            output,
            domain,
            autologon,
            tls_enabled,
            credssp_enabled,
            compression_enabled,
            compression_level,
            no_graphics_timeout_seconds,
            after_first_graphics_seconds,
        }
    };

    if !args.finish().is_empty() {
        anyhow::bail!("unexpected argument(s)");
    }

    Ok(action)
}

fn setup_logging() -> anyhow::Result<()> {
    use tracing::metadata::LevelFilter;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

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

fn run(config: RunConfig) -> anyhow::Result<()> {
    let connector_config = build_config(
        config.username,
        config.password,
        config.domain,
        config.compression_enabled,
        config.compression_level,
        config.autologon,
        config.tls_enabled,
        config.credssp_enabled,
    )?;

    let (connection_result, framed) = connect(connector_config, config.host, config.port).context("connect")?;
    info!(compression_type = ?connection_result.compression_type, "Negotiated compression");

    let mut image = DecodedImage::new(
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    let got_graphics = active_stage(
        connection_result,
        framed,
        &mut image,
        Duration::from_secs(config.no_graphics_timeout_seconds),
        Duration::from_secs(config.after_first_graphics_seconds),
    )
    .context("active stage")?;

    if !got_graphics {
        anyhow::bail!("no graphics updates received before capture timeout");
    }

    let img: image::ImageBuffer<image::Rgba<u8>, _> =
        image::ImageBuffer::from_raw(u32::from(image.width()), u32::from(image.height()), image.data())
            .context("invalid image")?;

    img.save(config.output).context("save image to disk")?;

    Ok(())
}

fn build_config(
    username: String,
    password: String,
    domain: Option<String>,
    compression_enabled: bool,
    compression_level: u32,
    autologon: bool,
    tls_enabled: bool,
    credssp_enabled: bool,
) -> anyhow::Result<connector::Config> {
    let compression_type = if compression_enabled {
        Some(compression_type_from_level(compression_level)?)
    } else {
        None
    };

    Ok(connector::Config {
        credentials: Credentials::UsernamePassword { username, password },
        domain,
        enable_tls: tls_enabled,
        enable_credssp: credssp_enabled,
        enable_standard_rdp_security: false,
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        connection_type: ConnectionType::Lan,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize {
            width: 1280,
            height: 1024,
        },
        monitor_layout: None,
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
        autologon,
        enable_audio_playback: false,
        enable_audio_capture: false,
        compression_type,
        pointer_software_rendering: true,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
        rdp_version: ironrdp::pdu::gcc::RdpVersion::V5_PLUS,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel::empty(),
    })
}

fn compression_type_from_level(level: u32) -> anyhow::Result<CompressionType> {
    match level {
        0 => Ok(CompressionType::K8),
        1 => Ok(CompressionType::K64),
        2 => Ok(CompressionType::Rdp6),
        3 => Ok(CompressionType::Rdp61),
        _ => anyhow::bail!("Invalid compression level. Valid values are 0, 1, 2, 3."),
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

    // Bound blocking reads during connection setup.
    tcp_stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .context("set TCP read timeout")?;

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
        connector,
        &mut upgraded_framed,
        &mut network_client,
        server_name.into(),
        server_public_key,
        None,
    )
    .and_then(connector::ConnectionOutcome::into_connected)
    .context("finalize connection")?;

    Ok((connection_result, upgraded_framed))
}

fn remaining_capture_timeout(
    now: std::time::Instant,
    stage_started_at: std::time::Instant,
    first_graphic_time: Option<std::time::Instant>,
    no_graphics_timeout: Duration,
    after_first_graphics: Duration,
) -> Option<Duration> {
    let (started_at, timeout) = first_graphic_time
        .map(|started_at| (started_at, after_first_graphics))
        .unwrap_or((stage_started_at, no_graphics_timeout));

    timeout
        .checked_sub(now.saturating_duration_since(started_at))
        .filter(|timeout| !timeout.is_zero())
}

fn active_stage(
    connection_result: ConnectionResult,
    mut framed: UpgradedFramed,
    image: &mut DecodedImage,
    no_graphics_timeout: Duration,
    after_first_graphics: Duration,
) -> anyhow::Result<bool> {
    let mut active_stage = ActiveStageBuilder {
        static_channels: connection_result.static_channels,
        user_channel_id: connection_result.user_channel_id,
        io_channel_id: connection_result.io_channel_id,
        message_channel_id: connection_result.message_channel_id,
        share_id: connection_result.share_id,
        compression_type: connection_result.compression_type,
        enable_server_pointer: connection_result.enable_server_pointer,
        pointer_software_rendering: connection_result.pointer_software_rendering,
    }
    .build();
    let stage_started_at = std::time::Instant::now();
    let mut first_graphic_time: Option<std::time::Instant> = None;

    'outer: loop {
        let Some(timeout) = remaining_capture_timeout(
            std::time::Instant::now(),
            stage_started_at,
            first_graphic_time,
            no_graphics_timeout,
            after_first_graphics,
        ) else {
            break 'outer;
        };

        framed
            .get_inner_mut()
            .0
            .get_mut()
            .set_read_timeout(Some(timeout))
            .context("set TCP read timeout")?;

        let (action, payload) = match framed.read_pdu() {
            Ok((action, payload)) => (action, payload),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                continue 'outer;
            }
            Err(e) => return Err(anyhow::Error::new(e).context("read frame")),
        };

        trace!(?action, frame_length = payload.len(), "Frame received");

        let outputs = active_stage.process(image, action, &payload, framed.last_read_at())?;

        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => framed.write_all(&frame).context("write response")?,
                ActiveStageOutput::GraphicsUpdate(_) if first_graphic_time.is_none() => {
                    first_graphic_time = Some(std::time::Instant::now());
                }
                ActiveStageOutput::Terminate(_) => break 'outer,
                _ => {}
            }
        }
    }

    Ok(first_graphic_time.is_some())
}

fn lookup_addr(hostname: &str, port: u16) -> anyhow::Result<core::net::SocketAddr> {
    use std::net::ToSocketAddrs as _;
    let addr = (hostname, port)
        .to_socket_addrs()?
        .next()
        .context("socket address not found")?;
    Ok(addr)
}

fn tls_upgrade(
    stream: TcpStream,
    server_name: String,
) -> anyhow::Result<(rustls::StreamOwned<rustls::ClientConnection, TcpStream>, Vec<u8>)> {
    // This example intentionally skips server-certificate verification to support self-signed certificates.
    warn!("TLS server certificates are not verified; use only with trusted servers");
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

    let server_name = server_name.try_into()?;

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

    debug!(subject = %cert.tbs_certificate().subject());

    let server_public_key = cert
        .tbs_certificate()
        .subject_public_key_info()
        .subject_public_key
        .as_bytes()
        .context("subject public key BIT STRING is not aligned")?
        .to_owned();

    Ok(server_public_key)
}

mod danger {
    use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use tokio_rustls::rustls::{DigitallySignedStruct, Error, SignatureScheme, pki_types};

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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::{Action, parse_args, remaining_capture_timeout};

    fn parse(args: &[&str]) -> anyhow::Result<Action> {
        parse_args(pico_args::Arguments::from_vec(
            args.iter().map(OsString::from).collect(),
        ))
    }

    #[test]
    fn screenshot_options_preserve_existing_defaults() -> anyhow::Result<()> {
        // oracle: Preserve the existing screenshot example defaults and keep TLS and autologon opt-in.
        let Action::Run {
            port,
            output,
            autologon,
            tls_enabled,
            credssp_enabled,
            compression_enabled,
            compression_level,
            no_graphics_timeout_seconds,
            after_first_graphics_seconds,
            ..
        } = parse(&["--host", "localhost", "-u", "user", "-p", "password"])?
        else {
            anyhow::bail!("expected run action");
        };

        assert_eq!(port, 3389);
        assert_eq!(output, PathBuf::from("out.png"));
        assert!(!autologon);
        assert!(!tls_enabled);
        assert!(credssp_enabled);
        assert!(compression_enabled);
        assert_eq!(compression_level, 3);
        assert_eq!(no_graphics_timeout_seconds, 3);
        assert_eq!(after_first_graphics_seconds, 8);

        Ok(())
    }

    #[test]
    fn screenshot_options_accept_explicit_security_compression_and_capture_values() -> anyhow::Result<()> {
        // oracle: The documented command-line options must override their defaults as requested.
        let Action::Run {
            autologon,
            tls_enabled,
            credssp_enabled,
            compression_enabled,
            compression_level,
            no_graphics_timeout_seconds,
            after_first_graphics_seconds,
            ..
        } = parse(&[
            "--host",
            "localhost",
            "-u",
            "user",
            "-p",
            "password",
            "--autologon",
            "true",
            "--tls-enabled",
            "true",
            "--credssp-enabled",
            "false",
            "--compression-enabled",
            "false",
            "--compression-level",
            "2",
            "--no-graphics-timeout-seconds",
            "5",
            "--after-first-graphics-seconds",
            "0",
        ])?
        else {
            anyhow::bail!("expected run action");
        };

        assert!(autologon);
        assert!(tls_enabled);
        assert!(!credssp_enabled);
        assert!(!compression_enabled);
        assert_eq!(compression_level, 2);
        assert_eq!(no_graphics_timeout_seconds, 5);
        assert_eq!(after_first_graphics_seconds, 0);

        Ok(())
    }

    #[test]
    fn screenshot_options_reject_zero_initial_graphics_timeout() {
        // oracle: A zero initial timeout cannot provide a graphics wait and is rejected as invalid input.
        let error = parse(&[
            "--host",
            "localhost",
            "-u",
            "user",
            "-p",
            "password",
            "--no-graphics-timeout-seconds",
            "0",
        ])
        .expect_err("zero timeout should be rejected");

        assert!(error.to_string().contains("must be greater than zero"));
    }

    #[test]
    fn capture_timeout_uses_no_graphics_deadline_until_first_update() {
        // oracle: The initial capture wait ends at its configured deadline when no graphics arrive.
        let stage_started_at = Instant::now();

        assert_eq!(
            remaining_capture_timeout(
                stage_started_at + Duration::from_secs(2),
                stage_started_at,
                None,
                Duration::from_secs(3),
                Duration::from_secs(8),
            ),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            remaining_capture_timeout(
                stage_started_at + Duration::from_secs(3),
                stage_started_at,
                None,
                Duration::from_secs(3),
                Duration::from_secs(8),
            ),
            None
        );
    }

    #[test]
    fn capture_timeout_uses_first_graphics_grace_period() {
        // oracle: The post-first-graphics wait starts at the first update, not at the active-stage start.
        let stage_started_at = Instant::now();
        let first_graphic_time = stage_started_at + Duration::from_secs(2);

        assert_eq!(
            remaining_capture_timeout(
                stage_started_at + Duration::from_secs(5),
                stage_started_at,
                Some(first_graphic_time),
                Duration::from_secs(3),
                Duration::from_secs(8),
            ),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            remaining_capture_timeout(
                first_graphic_time + Duration::from_secs(8),
                stage_started_at,
                Some(first_graphic_time),
                Duration::from_secs(3),
                Duration::from_secs(8),
            ),
            None
        );
    }
}
