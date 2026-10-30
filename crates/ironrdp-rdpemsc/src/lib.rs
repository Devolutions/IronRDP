#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

/// Channel name from [2.1] Transport.
///
/// [2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::MouseCursor";

pub mod client;
pub mod pdu;
pub mod server;

pub use client::MouseCursorClient;
pub use pdu::{CapabilitySet, CapsAdvertisePdu, CapsConfirmPdu, MouseCursorPdu, PointerShape, PointerUpdate};
pub use server::MouseCursorServer;
