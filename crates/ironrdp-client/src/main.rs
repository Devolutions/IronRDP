#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary

use anyhow::Context as _;
use ironrdp_client::app::App;
use ironrdp_client::config::{ClipboardType, Config};
use ironrdp_client::rdp::{DvcPipeProxyFactory, RdpClient, RdpInputEvent, RdpOutputEvent};
use tokio::runtime;
use tracing::debug;
use winit::event_loop::EventLoop;

fn main() -> anyhow::Result<()> {
    let mut config = Config::parse_args().context("CLI arguments parsing")?;

    setup_logging(config.log_file.as_deref()).context("unable to initialize logging")?;

    debug!("Initialize App");
    let event_loop = EventLoop::<RdpOutputEvent>::with_user_event().build()?;
    let event_loop_proxy = event_loop.create_proxy();
    let (input_event_sender, input_event_receiver) = RdpInputEvent::create_channel();
    let mut app = App::new(&event_loop, &input_event_sender).context("unable to initialize App")?;

    // TODO: get window size & scale factor from GUI/App
    let window_size = (1024, 768);
    config.connector.desktop_scale_factor = 0;
    config.connector.desktop_size.width = u16::try_from(window_size.0).unwrap();
    config.connector.desktop_size.height = u16::try_from(window_size.1).unwrap();

    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("unable to create tokio runtime")?;

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

            let cliprdr = WinClipboard::new(ClientClipboardMessageProxy::new(input_event_sender.clone()))?;

            let factory = cliprdr.backend_factory();
            _win_clipboard = cliprdr;
            Some(factory)
        }
        _ => None,
    };

    let dvc_pipe_proxy_factory = DvcPipeProxyFactory::new(input_event_sender);

    let client = RdpClient {
        config,
        event_loop_proxy,
        input_event_receiver,
        cliprdr_factory,
        dvc_pipe_proxy_factory,
    };

    debug!("Start RDP thread");
    std::thread::spawn(move || {
        rt.block_on(client.run());
    });

    debug!("Run App");
    event_loop.run_app(&mut app)?;
    Ok(())
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
