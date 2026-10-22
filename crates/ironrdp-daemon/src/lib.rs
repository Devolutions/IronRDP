#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

mod clipboard;
pub mod daemon;
#[cfg(windows)]
pub mod drives;
pub mod known_certificates;
pub mod logbuf;
pub mod now;
pub mod operations;
pub mod printer_cache;

mod ipc {
    pub(crate) use ironrdp_rpc::ipc::*;
}

mod transport {
    pub(crate) use ironrdp_rpc::transport::*;
}
