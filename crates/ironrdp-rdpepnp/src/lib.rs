#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

/// Dynamic virtual channel of the PNP Device Info Subprotocol, from [MS-RDPEPNP] section 2.1 "Transport".
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
pub const PNPDR_CHANNEL_NAME: &str = "PNPDR";

/// Dynamic virtual channel of the PNP Device I/O Subprotocol, from [MS-RDPEPNP] section 2.1 "Transport".
///
/// The server opens one instance of this channel for each file handle.
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
pub const FILE_REDIRECTOR_CHANNEL_NAME: &str = "FileRedirectorChannel";

pub mod client;
pub mod pdu;
