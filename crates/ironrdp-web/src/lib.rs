#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![cfg_attr(
    doc,
    doc(
        html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg"
    )
)]
#![allow(clippy::new_without_default)] // Default trait can’t be used by wasm consumer anyway.

// Silence the unused_crate_dependencies lint.
// These crates are added just to enable additional WASM features.
extern crate chrono as _;
extern crate getrandom as _;
extern crate getrandom2 as _;
extern crate getrandom4 as _;
extern crate time as _;

#[cfg(feature = "bench")]
mod bench;
mod canvas;
mod clipboard;
// EGFX (MS-RDPEGFX) H.264 graphics pipeline: WebCodecs decode + GPU present. Requires the
// `web_sys_unstable_apis` cfg (WebCodecs `VideoDecoder` + softblit's `VideoFrame` import), which the
// xtask web build always sets.
#[cfg(all(target_arch = "wasm32", web_sys_unstable_apis))]
mod egfx;
mod error;
mod image;
mod input;
mod network_client;
mod printer;
mod rdp_file;
mod session;

mod wasm_bridge {
    use tracing::debug;

    struct Api;

    impl iron_remote_desktop::RemoteDesktopApi for Api {
        type Session = crate::session::Session;
        type SessionBuilder = crate::session::SessionBuilder;
        type SessionTerminationInfo = crate::session::SessionTerminationInfo;
        type DeviceEvent = crate::input::DeviceEvent;
        type InputTransaction = crate::input::InputTransaction;
        type ClipboardData = crate::clipboard::ClipboardData;
        type ClipboardItem = crate::clipboard::ClipboardItem;
        type Error = crate::error::IronError;

        fn post_setup() {
            debug!("IronRDP is ready");
        }
    }

    iron_remote_desktop::make_bridge!(Api);
}
