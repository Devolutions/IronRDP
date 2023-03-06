#[macro_use]
extern crate log;

use anyhow::Context as _;
use ironrdp_client::config::Config;
use ironrdp_client::gui::GuiContext;
use ironrdp_client::rdp::{RdpClient, RdpInputEvent};
use tokio::runtime;

fn main() -> anyhow::Result<()> {
    let mut config = Config::parse_args().context("CLI arguments parsing")?;

    setup_logging(config.log_file.as_str()).context("Unable to initialize logging")?;

    debug!("Initialize GUI context");
    let gui = GuiContext::init().context("Unable to initialize GUI context")?;
    debug!("GUI context initialized");

    let window_size = gui.window.inner_size();
    config.input.width = u16::try_from(window_size.width).unwrap();
    config.input.height = u16::try_from(window_size.height).unwrap();

    let event_loop_proxy = gui.event_loop.create_proxy();

    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Unable to create tokio runtime")?;

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
