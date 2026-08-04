#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary

use core::num::NonZeroU32;
use std::sync::Arc;

use anyhow::Context as _;
use ironrdp::client::rdp::{RdpClient, RdpOutputEvent};
use ironrdp_daemon::daemon::{self, Daemon};
use ironrdp_propertyset::PropertySet;
use ironrdp_rpc::transport;
use ironrdp_viewer::app::{App, InputTarget, ViewerEvent};
use ironrdp_viewer::cli::ViewerConfig;
use tokio::runtime;
use tokio::sync::mpsc;
use tracing::debug;
use winit::dpi::PhysicalSize;
use winit::event_loop::EventLoop;

fn main() -> anyhow::Result<()> {
    let cli = ViewerConfig::parse_args().context("CLI arguments parsing")?;

    setup_logging(cli.log_file()).context("unable to initialize logging")?;

    if cli.rpc_mode() {
        return run_rpc(cli).context("RPC server");
    }

    let dump_rdp = cli.dump_rdp().map(ToOwned::to_owned);
    let config = cli.into_config().context("configuration")?;

    if let Some(dump_path) = dump_rdp {
        // Dump the effective, secret-stripped PropertySet observed from the built configuration.
        let content = ironrdp_rdpfile::write(config.properties());
        std::fs::write(&dump_path, &content).with_context(|| format!("failed to write {}", dump_path.display()))?;
        return Ok(());
    }

    debug!("Initialize App");
    let event_loop = EventLoop::<ViewerEvent>::with_user_event().build()?;
    let event_loop_proxy = event_loop.create_proxy();
    let (output_event_sender, mut output_event_receiver) = mpsc::channel::<RdpOutputEvent>(64);
    let initial_window_size = PhysicalSize::new(
        u32::from(config.connector().desktop_size.width),
        u32::from(config.connector().desktop_size.height),
    );

    let client = RdpClient::new(config, output_event_sender);
    let input_event_sender = client.input_sender();

    let mut app =
        App::new(&event_loop, &input_event_sender, initial_window_size).context("unable to initialize App")?;

    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("unable to create tokio runtime")?;

    // Forward output events from the library's mpsc channel to winit's `EventLoopProxy`.
    //
    // The library is winit-agnostic: it just emits `RdpOutputEvent`s on a plain
    // `tokio::sync::mpsc` channel. Bridging onto the GUI event loop is the binary's job.
    rt.spawn(async move {
        while let Some(event) = output_event_receiver.recv().await {
            if event_loop_proxy.send_event(ViewerEvent::Output(event)).is_err() {
                // The event loop is gone; nothing left to forward.
                break;
            }
        }
    });

    debug!("Start RDP thread");
    std::thread::spawn(move || {
        rt.block_on(client.run());
    });

    debug!("Run App");
    event_loop.run_app(&mut app)?;

    Ok(())
}

fn run_rpc(cli: ViewerConfig) -> anyhow::Result<()> {
    let endpoint = cli
        .rpc_endpoint()
        .map(|value| transport::endpoint_from_string(value.to_owned()))
        .unwrap_or_else(|| transport::default_endpoint_named("ironrdp-viewer"));

    debug!(%endpoint, "Initialize viewer RPC server");
    let event_loop = EventLoop::<ViewerEvent>::with_user_event().build()?;
    let event_loop_proxy = event_loop.create_proxy();
    let (notification_sender, mut notification_receiver) = mpsc::channel(1);
    let daemon = Arc::new(Daemon::with_overlay(PropertySet::new()).with_notification(notification_sender));

    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("unable to create tokio runtime")?;

    let frame_daemon = Arc::clone(&daemon);
    let frame_proxy = event_loop_proxy.clone();
    runtime.spawn(async move {
        while notification_receiver.recv().await.is_some() {
            let Some(frame) = frame_daemon.current_frame() else {
                continue;
            };
            let (Some(width), Some(height)) = (
                NonZeroU32::new(u32::from(frame.width)),
                NonZeroU32::new(u32::from(frame.height)),
            ) else {
                continue;
            };
            if frame_proxy
                .send_event(ViewerEvent::Frame {
                    buffer: frame.pixels,
                    width,
                    height,
                })
                .is_err()
            {
                break;
            }
        }
    });

    let server_daemon = Arc::clone(&daemon);
    let server_proxy = event_loop_proxy;
    std::thread::Builder::new()
        .name("ironrdp-viewer-rpc".to_owned())
        .spawn(move || {
            let server_runtime = runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("unable to create viewer RPC runtime");
            server_runtime.block_on(async move {
                if let Err(error) = daemon::serve(endpoint, server_daemon).await {
                    tracing::error!(error = format!("{error:#}"), "Viewer RPC server stopped with an error");
                }
                let _ = server_proxy.send_event(ViewerEvent::Shutdown);
            });
        })
        .context("unable to spawn viewer RPC server")?;

    let mut app = App::new_with_input_target(&event_loop, InputTarget::Rpc(daemon), PhysicalSize::new(1024, 768))
        .context("unable to initialize App")?;

    debug!("Run viewer RPC App");
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn setup_logging(log_file: Option<&str>) -> anyhow::Result<()> {
    use std::fs::OpenOptions;

    use tracing::metadata::LevelFilter;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .with_env_var("IRONRDP_LOG")
        .from_env_lossy();

    if let Some(log_file) = log_file {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file)
            .with_context(|| format!("couldn't open {log_file}"))?;
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(file)
            .compact();
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()
            .context("failed to set tracing global subscriber")?;
    } else {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .compact()
            .with_file(true)
            .with_line_number(true)
            .with_thread_ids(true)
            .with_target(false);
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()
            .context("failed to set tracing global subscriber")?;
    };

    Ok(())
}
