#[macro_use]
extern crate tracing;

use anyhow::Context as _;
use ironrdp_client::config::Config;
use ironrdp_client::gui::GuiContext;
use ironrdp_client::rdp::{RdpClient, RdpInputEvent};
use tokio::runtime;

fn main() -> anyhow::Result<()> {
    let mut config = Config::parse_args().context("CLI arguments parsing")?;

    setup_logging(config.log_file.as_str()).context("unable to initialize logging")?;

    debug!("Initialize GUI context");
    let gui = GuiContext::init().context("unable to initialize GUI context")?;
    debug!("GUI context initialized");

    let window_size = gui.window.inner_size();
    config.connector.desktop_size.width = u16::try_from(window_size.width).unwrap();
    config.connector.desktop_size.height = u16::try_from(window_size.height).unwrap();

    let event_loop_proxy = gui.event_loop.create_proxy();

    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("unable to create tokio runtime")?;

    let (input_event_sender, input_event_receiver) = RdpInputEvent::create_channel();

    let client = RdpClient {
        config,
        event_loop_proxy,
        input_event_receiver,
    };

    debug!("Start RDP thread");
    std::thread::spawn(move || {
        rt.block_on(client.run());
    });

    debug!("Run GUI");
    gui.run(input_event_sender);
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
        .with_context(|| format!("couldn’t open {log_file}"))?;

    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(false)
        .with_writer(file);

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .with_env_var("IRONRDP_LOG_LEVEL")
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(env_filter)
        .try_init()
        .context("failed to set tracing global subscriber")?;

    Ok(())
}
