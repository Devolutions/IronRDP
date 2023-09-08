#![allow(clippy::new_without_default)] // Default trait can’t be used by wasm consumer anyway

#[macro_use]
extern crate tracing;

mod canvas;
mod error;
mod image;
mod input;
mod network_client;
mod session;
mod websocket;

use wasm_bindgen::prelude::*;

// NOTE: #[wasm_bindgen(start)] didn’t work last time I tried
#[wasm_bindgen]
pub fn ironrdp_init(log_level: &str) {
    use tracing::Level;
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt::time::UtcTime;
    use tracing_subscriber::prelude::*;
    use tracing_web::MakeConsoleWriter;

    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    #[cfg(feature = "panic_hook")]
    console_error_panic_hook::set_once();

    if let Ok(level) = log_level.parse::<Level>() {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_timer(UtcTime::rfc_3339()) // std::time is not available in browsers
            .with_writer(MakeConsoleWriter);

        let level_filter = LevelFilter::from_level(level);

        tracing_subscriber::registry().with(fmt_layer).with(level_filter).init();

        debug!("IronRDP is ready");
    }
}

#[wasm_bindgen]
#[derive(Clone)]
pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}

#[wasm_bindgen]
impl DesktopSize {
    pub fn new(width: u16, height: u16) -> Self {
        DesktopSize { width, height }
    }
}
