#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

//! MS-RDPEI Input Virtual Channel Extension.

mod client;
pub mod pdu;
mod server;

pub use client::RdpeiClient;
pub use server::{RdpeiHandler, RdpeiServer};

/// Dynamic channel name for MS-RDPEI ([MS-RDPEI] 2.1).
///
/// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/1aab43bf-cab8-4f0a-9eb3-b83b8365e237
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Input";
