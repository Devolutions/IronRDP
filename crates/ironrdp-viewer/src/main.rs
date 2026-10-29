#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary

use anyhow::Context as _;
use core::sync::atomic::{AtomicBool, Ordering};
use ironrdp::client::output_channel::output_channel;
use ironrdp::client::rdp::{RdpClient, RdpOutputEvent};
use ironrdp_daemon::daemon::{self, Daemon};
use ironrdp_propertyset::PropertySet;
use ironrdp_rpc::transport;
use ironrdp_viewer::app::{App, RpcApp, ViewerEvent};
use ironrdp_viewer::cli::ViewerConfig;
use std::sync::Arc;
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

    if cli.list_usb_devices() {
        return ironrdp_viewer::cli::print_usb_devices();
    }

    if cli.list_audio_devices() {
        print_audio_devices();
        return Ok(());
    }

    let dump_rdp = cli.dump_rdp().map(ToOwned::to_owned);
    #[cfg(windows)]
    let printers = cli.printers().cloned();
    let named_drives = cli.named_drives().to_vec();
    #[cfg(windows)]
    let ports = cli.ports().clone();
    let config = cli.into_config().context("configuration")?;
    #[cfg(not(windows))]
    if !named_drives.is_empty() || !config.channels().rdpdr.drives.is_empty() {
        tracing::warn!("Ignoring drive redirection request; it is only available on Windows");
    }

    if let Some(dump_path) = dump_rdp {
        // Dump the effective, secret-stripped PropertySet observed from the built configuration.
        let content = ironrdp_rdpfile::write(config.properties());
        std::fs::write(&dump_path, &content).with_context(|| format!("failed to write {}", dump_path.display()))?;
        return Ok(());
    }

    debug!("Initialize App");
    let event_loop = EventLoop::<RdpOutputEvent>::with_user_event().build()?;
    let event_loop_proxy = event_loop.create_proxy();
    let (output_event_sender, mut output_event_receiver) = output_channel(64);
    let remote_app = config.connector().remote_application_mode;
    let initial_window_size = PhysicalSize::new(
        u32::from(config.connector().desktop_size.width),
        u32::from(config.connector().desktop_size.height),
    );

    #[cfg(windows)]
    let enable_smartcard = config.channels().rdpdr.smartcard;
    #[cfg(windows)]
    let enable_clipboard = config.channels().clipboard == ironrdp::client::config::ClipboardType::Enable;
    #[cfg(windows)]
    let server = config.destination().to_string();
    #[cfg(windows)]
    let drives = ironrdp_viewer::drives::ViewerDrives::from_rdpdr_config(&config.channels().rdpdr, named_drives);
    let client = RdpClient::new(config, output_event_sender);
    #[cfg(windows)]
    let (client, _drive_watcher) =
        attach_windows_rdpdr_backend(client, enable_smartcard, printers.as_ref(), &ports, &drives, &server)?;
    // Kept alive, on this thread, until the event loop returns.
    #[cfg(windows)]
    let (client, _clipboard) = attach_windows_clipboard(client, enable_clipboard)?;
    let input_event_sender = client.input_sender();

    let mut app = App::new(&event_loop, &input_event_sender, initial_window_size, remote_app)
        .context("unable to initialize App")?;

    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("unable to create tokio runtime")?;

    // Forward output events from the library's output channel to winit's `EventLoopProxy`.
    //
    // The library is winit-agnostic: it just emits `RdpOutputEvent`s on
    // `output_channel`'s `OutputEventSender`/`OutputEventReceiver` pair (a plain
    // `mpsc` channel for correctness-sensitive events, `watch` channels underneath
    // for high-frequency display state; see `RdpOutputEvent::drop_policy`).
    // Bridging onto the GUI event loop is the binary's job.
    let output_forwarder = rt.spawn(async move {
        while let Some(event) = output_event_receiver.recv().await {
            if event_loop_proxy.send_event(event).is_err() {
                // The event loop is gone; nothing left to forward.
                break;
            }
        }
    });

    debug!("Start RDP thread");
    std::thread::spawn(move || {
        rt.block_on(client.run());
        // Deliver the final failure or termination event before dropping the runtime.
        if let Err(error) = rt.block_on(output_forwarder) {
            tracing::warn!(%error, "RDP output forwarder failed");
        }
    });

    debug!("Run App");
    event_loop.run_app(&mut app)?;

    Ok(())
}

fn run_rpc(cli: ViewerConfig) -> anyhow::Result<()> {
    let endpoint = cli
        .rpc_endpoint()
        .map(|value| transport::endpoint_from_string(value.to_owned()))
        .unwrap_or_else(transport::default_endpoint);

    debug!(%endpoint, "Initialize viewer RPC server");
    let event_loop = EventLoop::<ViewerEvent>::with_user_event().build()?;
    let event_loop_proxy = event_loop.create_proxy();
    let (notification_sender, mut notification_receiver) = mpsc::channel(1);
    let daemon = Arc::new(Daemon::with_overlay(PropertySet::new()).with_notification(notification_sender));
    let frame_wakeup = Arc::new(AtomicBool::new(false));

    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("unable to create tokio runtime")?;

    let frame_proxy = event_loop_proxy.clone();
    let frame_wakeup_sender = Arc::clone(&frame_wakeup);
    runtime.spawn(async move {
        while notification_receiver.recv().await.is_some() {
            if !frame_wakeup_sender.swap(true, Ordering::AcqRel)
                && frame_proxy.send_event(ViewerEvent::FrameAvailable).is_err()
            {
                break;
            }
        }
    });

    let server_daemon = Arc::clone(&daemon);
    let server_proxy = event_loop_proxy;
    let server = std::thread::Builder::new()
        .name("ironrdp-viewer-rpc".to_owned())
        .spawn(move || -> anyhow::Result<()> {
            let server_runtime = runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("unable to create viewer RPC runtime")?;
            let result = server_runtime.block_on(daemon::serve(endpoint, server_daemon));
            let _ = server_proxy.send_event(ViewerEvent::Shutdown);
            result
        })
        .context("unable to spawn viewer RPC server")?;

    let mut app = RpcApp::new(
        &event_loop,
        Arc::clone(&daemon),
        frame_wakeup,
        PhysicalSize::new(1024, 768),
    )
    .context("unable to initialize App")?;

    debug!("Run viewer RPC App");
    let event_loop_result = event_loop.run_app(&mut app);
    daemon.shutdown();
    let server_result = server
        .join()
        .map_err(|_| anyhow::anyhow!("viewer RPC server thread panicked"))?;

    event_loop_result?;
    server_result?;
    Ok(())
}

/// Creates the Windows clipboard backend on this thread, the winit one, when clipboard redirection
/// is enabled.
///
/// The clipboard listens through a hidden window, so it needs a thread with a message loop,
/// which the RDP thread is not; pasting remote files also needs OLE on a single-threaded
/// apartment, initialized here before winit creates its window (winit's own `OleInitialize`
/// then succeeds as a no-op).
#[cfg(windows)]
fn attach_windows_clipboard(
    client: RdpClient,
    enabled: bool,
) -> anyhow::Result<(RdpClient, Option<ironrdp_cliprdr_native::WinClipboard>)> {
    if !enabled {
        return Ok((client, None));
    }

    // SAFETY: Called on the main thread before any COM use; it lasts for the process.
    unsafe { windows::Win32::System::Ole::OleInitialize(None) }.context("unable to initialize OLE")?;

    let clipboard = ironrdp_cliprdr_native::WinClipboard::with_file_transfer(
        ironrdp_viewer::clipboard::ClientClipboardMessageProxy::new(client.input_sender()),
        true,
    )
    .context("unable to initialize the Windows clipboard")?;
    let client = client.with_cliprdr_backend_factory(clipboard.backend_factory());

    Ok((client, Some(clipboard)))
}

/// Attaches the Windows RDPDR backend when drive, smartcard, printer or port redirection is enabled.
///
/// Printer redirection announces the selected queues; when none exists, the session continues
/// without a printer. Server printer settings persist per server in
/// [`ironrdp_daemon::printer_cache::directory`]. Ports are enumerated when the session connects,
/// and requested ports that are not present are skipped. With dynamic drives, the returned
/// watcher announces drives connected during the session until it is dropped.
#[cfg(windows)]
fn attach_windows_rdpdr_backend(
    client: RdpClient,
    enable_smartcard: bool,
    printers: Option<&ironrdp_viewer::cli::PrinterOptions>,
    ports: &ironrdp_viewer::cli::PortOptions,
    drives: &ironrdp_viewer::drives::ViewerDrives,
    server: &str,
) -> anyhow::Result<(RdpClient, Option<ironrdp_rdpdr_native::LogicalDriveWatcher>)> {
    use ironrdp_rdpdr_native::{PortSelection, WindowsPortRedirection};
    use ironrdp_viewer::cli::PortChoice;

    if !enable_smartcard && printers.is_none() && !ports.is_enabled() && drives.is_empty() {
        return Ok((client, None));
    }

    let selection = |choice: &PortChoice| match choice {
        PortChoice::None => PortSelection::None,
        PortChoice::All => PortSelection::All,
        PortChoice::Named(names) => PortSelection::Named(names.clone()),
    };
    let (factory, drive_watcher) = drives
        .backend_factory(&client.input_sender())
        .context("invalid rdpdr configuration")?;
    let factory = factory
        .with_smartcard(enable_smartcard)
        .with_printer_redirection(printers.map(windows_printer_redirection).unwrap_or_default())
        .with_port_redirection(WindowsPortRedirection {
            serial: selection(&ports.serial),
            parallel: selection(&ports.parallel),
        });
    let factory = ironrdp_daemon::printer_cache::with_store(factory, server);
    return Ok((client.with_rdpdr_backend_factory(Box::new(factory)), drive_watcher));

    fn windows_printer_redirection(
        options: &ironrdp_viewer::cli::PrinterOptions,
    ) -> ironrdp_rdpdr_native::WindowsPrinterRedirection {
        use ironrdp_rdpdr_native::{WindowsPrinterRedirection, WindowsPrinterSelection};
        use ironrdp_viewer::cli::PrinterSelection;

        let selection = match &options.selection {
            PrinterSelection::Default => WindowsPrinterSelection::Default,
            PrinterSelection::All => WindowsPrinterSelection::All,
            PrinterSelection::Named(queues) => WindowsPrinterSelection::Named(queues.clone()),
        };
        options.drivers.iter().fold(
            WindowsPrinterRedirection::new(selection).with_default_queue(options.default_queue.clone()),
            |printers, (queue, driver)| printers.with_driver(queue.clone(), driver.clone()),
        )
    }
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

#[expect(clippy::print_stdout, reason = "--list-audio-devices reports to the terminal")]
fn print_audio_devices() {
    use ironrdp::client::audio::{Direction, list_devices};

    for (direction, label) in [
        (Direction::Output, "Audio output devices"),
        (Direction::Input, "Audio input devices"),
    ] {
        println!("{label}:");
        for device in list_devices(direction) {
            let marker = if device.is_default { " (default)" } else { "" };
            println!(
                "  {}{marker}
    {}",
                device.name, device.id
            );
        }
    }
}
