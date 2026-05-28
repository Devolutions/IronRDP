#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary

use anyhow::Context as _;
use ironrdp_client::config::ClipboardType;
use ironrdp_client::rdp::{DvcPipeProxyFactory, RdpClient, RdpInputEvent, RdpOutputEvent};
use ironrdp_viewer::app::App;
use ironrdp_viewer::config::{build_config, parse_inputs};
use tokio::runtime;
use tokio::sync::mpsc;
use tracing::debug;
use winit::dpi::PhysicalSize;
use winit::event_loop::EventLoop;

fn main() -> anyhow::Result<()> {
    let parsed = parse_inputs().context("CLI arguments parsing")?;

    if let Some(dump_path) = &parsed.args.dump_rdp {
        let content = ironrdp_rdpfile::write(&parsed.properties);
        std::fs::write(dump_path, &content).with_context(|| format!("failed to write {}", dump_path.display()))?;
        return Ok(());
    }

    setup_logging(parsed.args.log_file.as_deref()).context("unable to initialize logging")?;

    debug!("Initialize App");
    let event_loop = EventLoop::<RdpOutputEvent>::with_user_event().build()?;
    let event_loop_proxy = event_loop.create_proxy();
    let (input_event_sender, input_event_receiver) = RdpInputEvent::create_channel();
    let (output_event_sender, mut output_event_receiver) = mpsc::channel::<RdpOutputEvent>(64);

    // NOTE: we need to keep `win_clipboard` alive, otherwise it will be dropped before IronRDP
    // starts and clipboard functionality will not be available.
    #[cfg(windows)]
    let _win_clipboard;

    let mut config = build_config(parsed).context("configuration")?;

    let cliprdr_factory: Option<Box<dyn ironrdp::cliprdr::backend::CliprdrBackendFactory + Send>> =
        match config.clipboard_type {
            ClipboardType::Stub => {
                use ironrdp_cliprdr_native::StubClipboard;

                let cliprdr = StubClipboard::new();
                Some(cliprdr.backend_factory())
            }
            ClipboardType::Enable => {
                #[cfg(windows)]
                {
                    use ironrdp_cliprdr_native::WinClipboard;
                    use ironrdp_viewer::clipboard::ClientClipboardMessageProxy;

                    let cliprdr = WinClipboard::new(ClientClipboardMessageProxy::new(input_event_sender.clone()))?;
                    let factory = cliprdr.backend_factory();
                    _win_clipboard = cliprdr;
                    Some(factory)
                }
                #[cfg(not(windows))]
                {
                    use ironrdp_cliprdr_native::StubClipboard;

                    let cliprdr = StubClipboard::new();
                    Some(cliprdr.backend_factory())
                }
            }
            ClipboardType::Disable => None,
        };

    if let Some(factory) = cliprdr_factory {
        config.clipboard_backend = Some(factory);
    }

    let initial_window_size = PhysicalSize::new(
        u32::from(config.connector.desktop_size.width),
        u32::from(config.connector.desktop_size.height),
    );
    let mut app = App::new(
        &event_loop,
        &input_event_sender,
        config.fake_events_interval,
        initial_window_size,
    )
    .context("unable to initialize App")?;

    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("unable to create tokio runtime")?;

    let dvc_pipe_proxy_factory = DvcPipeProxyFactory::new(input_event_sender);

    let client = RdpClient {
        config,
        output_event_sender,
        input_event_receiver,
        dvc_pipe_proxy_factory,
    };

    // Forward output events from the library's mpsc channel to winit's `EventLoopProxy`.
    rt.spawn(async move {
        while let Some(event) = output_event_receiver.recv().await {
            if event_loop_proxy.send_event(event).is_err() {
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
