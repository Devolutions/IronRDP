#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary

#[macro_use]
extern crate tracing;

use anyhow::Context as _;
use ironrdp_client::config::{ClipboardType, Config};
use ironrdp_client::gui::GuiContext;
use ironrdp_client::rdp::{RdpClient, RdpInputEvent};
use tokio::runtime;

fn main() -> anyhow::Result<()> {
    let mut config = Config::parse_args().context("CLI arguments parsing")?;

    setup_logging(config.log_file.as_deref()).context("unable to initialize logging")?;

    debug!("Initialize GUI context");
    let gui = GuiContext::init().context("unable to initialize GUI context")?;
    debug!("GUI context initialized");

    let window_size = (1024, 768); // TODO: get window size from GUI
    config.connector.desktop_scale_factor = 0; // TODO: should this be `(gui.window().scale_factor() * 100.0) as u32`?
    config.connector.desktop_size.width = u16::try_from(window_size.0).unwrap();
    config.connector.desktop_size.height = u16::try_from(window_size.1).unwrap();

    let event_loop_proxy = gui.create_event_proxy();

    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("unable to create tokio runtime")?;

    let (input_event_sender, input_event_receiver) = RdpInputEvent::create_channel();

    // NOTE: we need to keep `win_clipboard` alive, otherwise it will be dropped before IronRDP
    // starts and clipboard functionality will not be available.
    #[cfg(windows)]
    let _win_clipboard;

    let cliprdr_factory = match config.clipboard_type {
        ClipboardType::Stub => {
            use ironrdp_cliprdr_native::StubClipboard;

            let cliprdr = StubClipboard::new();
            let factory = cliprdr.backend_factory();
            Some(factory)
        }
        #[cfg(windows)]
        ClipboardType::Windows => {
            use ironrdp_client::clipboard::ClientClipboardMessageProxy;
            use ironrdp_cliprdr_native::WinClipboard;

            // SAFETY: provided window handle from `winit` is valid and is guaranteed to be alive
            // while the gui window is still open.
            let cliprdr = WinClipboard::new(ClientClipboardMessageProxy::new(input_event_sender.clone()))?;

            let factory = cliprdr.backend_factory();
            _win_clipboard = cliprdr;
            Some(factory)
        }
        _ => None,
    };

    let client = RdpClient {
        config,
        event_loop_proxy,
        input_event_receiver,
        cliprdr_factory,
    };

    debug!("Start RDP thread");
    std::thread::spawn(move || {
        rt.block_on(client.run());
    });

    debug!("Run GUI");
    gui.run(input_event_sender)
}

fn setup_logging(log_file: Option<&str>) -> anyhow::Result<()> {
    use std::fs::OpenOptions;

    use tracing::metadata::LevelFilter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .with_env_var("IRONRDP_LOG")
        .from_env_lossy();

    if let Some(log_file) = log_file {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file)
            .with_context(|| format!("couldn’t open {log_file}"))?;
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
