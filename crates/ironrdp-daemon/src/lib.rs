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

/// Applies companion metadata from a sibling `<rdp_file>.json` file onto `properties` when not
/// already set in the `.rdp` file.
pub fn apply_rdp_launch_companion(rdp_file: &std::path::Path, properties: &mut ironrdp_propertyset::PropertySet) {
    ironrdp_client::config::ConfigBuilder::apply_rdp_launch_companion(rdp_file, properties);
}

mod ipc {
    pub(crate) use ironrdp_rpc::ipc::*;
}

mod transport {
    pub(crate) use ironrdp_rpc::transport::*;
}
