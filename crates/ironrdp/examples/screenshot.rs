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
use std::io::{self, Read, Write as _};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Instant;

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
use tracing::{debug, info, trace};

const HELP: &str = "\
USAGE:
  cargo run --example=screenshot -- --host <HOSTNAME> -u/--username <USERNAME> -p/--password <PASSWORD>
                                    [--port <PORT>] [-o/--output <OUTPUT_FILE>] [-d/--domain <DOMAIN>]
                                    [--autologon <true|false>] [--tls-enabled <true|false>]
                                    [--credssp-enabled <true|false>]
                                    [--compression-enabled <true|false>] [--compression-level <0..3>]
                                    [--no-graphics-timeout-seconds <SECONDS>]
                                    [--after-first-graphics-seconds <SECONDS>]

TLS validates server certificates against the platform certificate store.

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
        Action::Run(config) => {
            info!(
                host = config.host,
                port = config.port,
                username = config.username,
                output = %config.output.display(),
                domain = config.domain,
                autologon = config.autologon,
                tls_enabled = config.tls_enabled,
                credssp_enabled = config.credssp_enabled,
                compression_enabled = config.compression_enabled,
                compression_level = config.compression_level,
                no_graphics_timeout_seconds = config.no_graphics_timeout_seconds,
                after_first_graphics_seconds = config.after_first_graphics_seconds,
                "run"
            );
            run(config)
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
    Run(RunConfig),
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

        Action::Run(RunConfig {
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
        config.autologon,
        config.tls_enabled,
        config.credssp_enabled,
        if config.compression_enabled {
            Some(compression_type_from_level(config.compression_level)?)
        } else {
            None
        },
    );

    let (connection_result, framed) = connect(connector_config, config.host, config.port).context("connect")?;
    info!(compression_type = ?connection_result.compression_type, "Negotiated compression");

    let mut image = DecodedImage::new(
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    active_stage(
        connection_result,
        framed,
        &mut image,
        Duration::from_secs(config.no_graphics_timeout_seconds),
        Duration::from_secs(config.after_first_graphics_seconds),
    )
    .context("active stage")?;

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
    autologon: bool,
    tls_enabled: bool,
    credssp_enabled: bool,
    compression_type: Option<CompressionType>,
) -> connector::Config {
    connector::Config {
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
    }
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

struct DeadlineTcpStream {
    stream: TcpStream,
    /// INVARIANT: Every read shares this absolute deadline until it is explicitly replaced.
    read_deadline: Option<Instant>,
}

impl DeadlineTcpStream {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            read_deadline: None,
        }
    }

    fn set_read_deadline(&mut self, deadline: Instant) {
        self.read_deadline = Some(deadline);
    }
}

impl Read for DeadlineTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(deadline) = self.read_deadline {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "read deadline elapsed"));
            }

            self.stream.set_read_timeout(Some(timeout))?;
        }

        self.stream.read(buf)
    }
}

impl io::Write for DeadlineTcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

type UpgradedFramed = ironrdp_blocking::Framed<rustls::StreamOwned<rustls::ClientConnection, DeadlineTcpStream>>;

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

fn capture_deadline(started_at: Instant, timeout: Duration) -> anyhow::Result<Instant> {
    started_at.checked_add(timeout).context("capture deadline is too large")
}

fn active_stage(
    connection_result: ConnectionResult,
    mut framed: UpgradedFramed,
    image: &mut DecodedImage,
    no_graphics_timeout: Duration,
    after_first_graphics: Duration,
) -> anyhow::Result<()> {
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
    capture_frames(
        &mut framed,
        no_graphics_timeout,
        after_first_graphics,
        &mut |stream, deadline| stream.get_mut().set_read_deadline(deadline),
        &mut Instant::now,
        &mut |action, payload, received_at| Ok(active_stage.process(image, action, payload, received_at)?),
    )
}

type FrameProcessor<'a> = dyn FnMut(ironrdp_pdu::Action, &[u8], Option<connector::MonotonicInstant>) -> anyhow::Result<Vec<ActiveStageOutput>>
    + 'a;

fn capture_frames<S: Read + io::Write>(
    framed: &mut ironrdp_blocking::Framed<S>,
    no_graphics_timeout: Duration,
    after_first_graphics: Duration,
    set_deadline: &mut dyn FnMut(&mut S, Instant),
    now: &mut dyn FnMut() -> Instant,
    process: &mut FrameProcessor<'_>,
) -> anyhow::Result<()> {
    let mut got_graphics = false;
    let mut deadline = capture_deadline(now(), no_graphics_timeout)?;

    // INVARIANT: Only the first graphics update replaces the initial deadline with that instant plus the grace period.
    'outer: loop {
        if deadline <= now() {
            break 'outer;
        }

        set_deadline(framed.get_inner_mut().0, deadline);

        let (action, payload) = match framed.read_pdu() {
            Ok((action, payload)) => (action, payload),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                continue 'outer;
            }
            Err(e) if got_graphics => {
                // Server closed without TLS close_notify; save what we have
                info!("Connection closed ({}); saving received graphics", e);
                break 'outer;
            }
            Err(e) => return Err(anyhow::Error::new(e).context("read frame")),
        };

        if deadline <= now() {
            break 'outer;
        }

        trace!(?action, frame_length = payload.len(), "Frame received");

        let outputs = process(action, &payload, framed.last_read_at())?;

        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => framed.write_all(&frame).context("write response")?,
                ActiveStageOutput::GraphicsUpdate(_) if !got_graphics => {
                    got_graphics = true;
                    deadline = capture_deadline(now(), after_first_graphics)?;
                }
                ActiveStageOutput::Terminate(_) => break 'outer,
                _ => {}
            }
        }
    }

    if !got_graphics {
        anyhow::bail!("no graphics updates received before capture timeout");
    }

    Ok(())
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
) -> anyhow::Result<(
    rustls::StreamOwned<rustls::ClientConnection, DeadlineTcpStream>,
    Vec<u8>,
)> {
    let config = std::sync::Arc::new(
        ironrdp_tls::rustls_client_config(ironrdp_tls::CertificateValidation::Strict, &server_name, None)
            .context("build TLS config")?,
    );

    let server_name = server_name.try_into()?;

    let client = rustls::ClientConnection::new(config, server_name)?;

    let mut tls_stream = rustls::StreamOwned::new(client, DeadlineTcpStream::new(stream));

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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::io::{self, Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{Action, ActiveStageOutput, DeadlineTcpStream, capture_frames, parse_args};

    fn parse(args: &[&str]) -> anyhow::Result<Action> {
        parse_args(pico_args::Arguments::from_vec(
            args.iter().map(OsString::from).collect(),
        ))
    }

    #[test]
    fn screenshot_options_preserve_existing_defaults() -> anyhow::Result<()> {
        // oracle: mutant changing the screenshot example's preserved defaults.
        let Action::Run(super::RunConfig {
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
        }) = parse(&["--host", "localhost", "-u", "user", "-p", "password"])?
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
        // oracle: mutant changing the chosen initial-wait or grace-period defaults.
        assert_eq!(no_graphics_timeout_seconds, 3);
        assert_eq!(after_first_graphics_seconds, 8);

        Ok(())
    }

    #[test]
    fn screenshot_options_accept_explicit_security_compression_and_capture_values() -> anyhow::Result<()> {
        // oracle: mutant ignoring explicit screenshot option values.
        let Action::Run(super::RunConfig {
            autologon,
            tls_enabled,
            credssp_enabled,
            compression_enabled,
            compression_level,
            no_graphics_timeout_seconds,
            after_first_graphics_seconds,
            ..
        }) = parse(&[
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
        // oracle: mutant accepting a zero initial graphics wait.
        parse(&[
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
    }

    #[test]
    fn screenshot_options_reject_unexpected_arguments() {
        // oracle: mutant silently ignoring an unknown command-line option.
        parse(&["--host", "localhost", "-u", "user", "-p", "password", "--bogus"])
            .expect_err("unknown options should be rejected");
    }

    #[test]
    fn capture_without_graphics_retries_early_timeouts_then_fails_at_initial_deadline() {
        // oracle: mutant treating an early socket timeout as capture expiration or succeeding without graphics.
        let start = Instant::now();
        let (result, deadlines, processed) = run_capture_script(
            start,
            vec![
                (Duration::from_secs(1), Err(io::ErrorKind::TimedOut.into())),
                (Duration::from_secs(1), Ok(())),
                (Duration::from_secs(1), Err(io::ErrorKind::TimedOut.into())),
            ],
            vec![false],
            Duration::from_secs(8),
        );

        assert!(result.is_err());
        assert_eq!(processed, 1);
        assert!(!deadlines.is_empty());
        assert!(
            deadlines
                .iter()
                .all(|deadline| *deadline == start + Duration::from_secs(3))
        );
    }

    #[test]
    fn capture_switches_to_first_graphics_grace_without_extending_it_on_later_updates() {
        // oracle: mutant extending the capture deadline after a later graphics update.
        let start = Instant::now();
        let (result, deadlines, processed) = run_capture_script(
            start,
            vec![
                (Duration::from_secs(2), Ok(())),
                (Duration::from_secs(2), Ok(())),
                (Duration::from_secs(6), Err(io::ErrorKind::TimedOut.into())),
            ],
            vec![true, true],
            Duration::from_secs(8),
        );

        result.expect("graphics should allow a capture after the grace period");
        assert_eq!(processed, 2);
        assert_eq!(deadlines.first(), Some(&(start + Duration::from_secs(3))));
        assert!(deadlines.contains(&(start + Duration::from_secs(10))));
        assert!(deadlines.iter().all(|deadline| {
            *deadline == start + Duration::from_secs(3) || *deadline == start + Duration::from_secs(10)
        }));
    }

    #[test]
    fn capture_with_zero_grace_stops_after_first_graphics() {
        // oracle: mutant reading another frame after the first graphics update with zero grace.
        let start = Instant::now();
        let (result, deadlines, processed) = run_capture_script(
            start,
            vec![(Duration::from_secs(2), Ok(()))],
            vec![true],
            Duration::ZERO,
        );

        result.expect("the first graphics update should be captured");
        assert_eq!(processed, 1);
        assert_eq!(deadlines, vec![start + Duration::from_secs(3)]);
    }

    #[test]
    fn capture_rejects_first_frame_completed_at_initial_deadline() {
        // oracle: mutant starting a grace period for a first frame completed at the initial deadline.
        let start = Instant::now();
        let (result, deadlines, processed) = run_capture_script(
            start,
            vec![(Duration::from_secs(3), Ok(()))],
            vec![true],
            Duration::from_secs(8),
        );

        assert!(result.is_err());
        assert_eq!(processed, 0);
        assert!(!deadlines.is_empty());
        assert!(
            deadlines
                .iter()
                .all(|deadline| *deadline == start + Duration::from_secs(3))
        );
    }

    #[test]
    fn capture_skips_frame_completed_after_grace_deadline() {
        // oracle: mutant processing a frame completed after the grace deadline.
        let start = Instant::now();
        let (result, deadlines, processed) = run_capture_script(
            start,
            vec![(Duration::from_secs(2), Ok(())), (Duration::from_secs(9), Ok(()))],
            vec![true, true],
            Duration::from_secs(8),
        );

        result.expect("the earlier graphics update should still be captured");
        assert_eq!(processed, 1);
        assert_eq!(deadlines.first(), Some(&(start + Duration::from_secs(3))));
        assert!(deadlines.contains(&(start + Duration::from_secs(10))));
        assert!(deadlines.iter().all(|deadline| {
            *deadline == start + Duration::from_secs(3) || *deadline == start + Duration::from_secs(10)
        }));
    }

    #[test]
    fn deadline_tcp_stream_enforces_one_deadline_across_fragmented_reads() -> anyhow::Result<()> {
        // oracle: mutant restarting the full timeout after each partial transport read.
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let server_addr = listener.local_addr()?;
        let (ready_sender, ready_receiver) = mpsc::channel();
        let (start_sender, start_receiver) = mpsc::channel();

        let server = thread::spawn(move || -> io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            ready_sender.send(()).map_err(io::Error::other)?;
            start_receiver.recv().map_err(io::Error::other)?;
            stream.write_all(&[1])?;
            thread::sleep(Duration::from_millis(120));
            stream.write_all(&[2])?;
            thread::sleep(Duration::from_millis(120));
            let _ = stream.write_all(&[3]);
            Ok(())
        });

        let mut stream = DeadlineTcpStream::new(TcpStream::connect(server_addr)?);
        ready_receiver.recv().map_err(io::Error::other)?;
        stream.set_read_deadline(Instant::now() + Duration::from_millis(200));
        start_sender.send(()).map_err(io::Error::other)?;

        let mut bytes = [0; 3];
        let error = stream
            .read_exact(&mut bytes)
            .expect_err("fragmented read should time out");

        assert!(matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
        server.join().expect("server thread should finish")?;

        Ok(())
    }

    struct ScriptedStream {
        now: Rc<Cell<Instant>>,
        steps: VecDeque<(Duration, io::Result<()>)>,
    }

    impl io::Read for ScriptedStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let (elapsed, result) = self.steps.pop_front().expect("unexpected extra frame read");
            self.now.set(self.now.get() + elapsed);
            result?;
            buf[..2].copy_from_slice(&[0x00, 0x02]);
            Ok(2)
        }
    }

    impl io::Write for ScriptedStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn run_capture_script(
        start: Instant,
        steps: Vec<(Duration, io::Result<()>)>,
        updates: Vec<bool>,
        grace: Duration,
    ) -> (anyhow::Result<()>, Vec<Instant>, usize) {
        let now = Rc::new(Cell::new(start));
        let mut framed = ironrdp_blocking::Framed::new(ScriptedStream {
            now: Rc::clone(&now),
            steps: steps.into(),
        });
        let mut deadlines = Vec::new();
        let mut updates: VecDeque<_> = updates.into();
        let mut processed = 0;
        let result = capture_frames(
            &mut framed,
            Duration::from_secs(3),
            grace,
            &mut |_, deadline| deadlines.push(deadline),
            &mut || now.get(),
            &mut |_, _, _| {
                processed += 1;
                Ok(if updates.pop_front().expect("unexpected frame processing") {
                    vec![ActiveStageOutput::GraphicsUpdate(
                        ironrdp_pdu::geometry::InclusiveRectangle {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        },
                    )]
                } else {
                    Vec::new()
                })
            },
        );
        (result, deadlines, processed)
    }
}
