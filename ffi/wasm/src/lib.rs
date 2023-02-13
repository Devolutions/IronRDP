#![allow(clippy::drop_non_drop)] // there are false positives in this file
#![allow(clippy::new_without_default)] // Default trait can’t be used by wasm consumer anyway

#[macro_use]
extern crate log;

mod error;
mod image;
mod input;
mod network_client;
mod session;
mod websocket;

use wasm_bindgen::prelude::*;

// TODO: proper error reporting

// NOTE: #[wasm_bindgen(start)] didn’t work last time I tried
#[wasm_bindgen]
pub fn ironrdp_init(log_level: &str) {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();

    if let Ok(level) = log_level.parse::<log::Level>() {
        console_log::init_with_level(level).unwrap();
    }
}

#[wasm_bindgen]
#[derive(Clone)]
pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}
