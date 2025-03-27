#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![allow(clippy::new_without_default)] // Default trait can’t be used by wasm consumer anyway
#![allow(unsafe_op_in_unsafe_fn)] // We can’t control code generated by `wasm-bindgen`

// Silence the unused_crate_dependencies lint
// These crates are added just to enable additional WASM features
extern crate chrono as _;
extern crate getrandom as _;
extern crate time as _;

#[macro_use]
extern crate tracing;

mod canvas;
mod clipboard;
mod error;
mod image;
mod input;
mod network_client;
mod session;

use wasm_bindgen::prelude::*;

compile_error!("do not compile ironrdp-web in this branch");

#[wasm_bindgen]
pub fn ironrdp_init(log_level: &str) {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    #[cfg(feature = "panic_hook")]
    console_error_panic_hook::set_once();

    if let Ok(level) = log_level.parse::<tracing::Level>() {
        set_logger_once(level);
    }
}

fn set_logger_once(level: tracing::Level) {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt::time::UtcTime;
    use tracing_subscriber::prelude::*;
    use tracing_web::MakeConsoleWriter;

    static INIT: std::sync::Once = std::sync::Once::new();

    INIT.call_once(|| {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_timer(UtcTime::rfc_3339()) // std::time is not available in browsers
            .with_writer(MakeConsoleWriter);

        let level_filter = LevelFilter::from_level(level);

        tracing_subscriber::registry().with(fmt_layer).with(level_filter).init();

        debug!("IronRDP is ready");
    })
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
