#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

/// Name of the channel carrying server-to-client PDUs ([2.1] Transport).
///
/// [2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
pub const SERVER_TO_CLIENT_CHANNEL_NAME: &str = "TextInput_ServerToClientDVC";

/// Name of the channel carrying client-to-server PDUs ([2.1] Transport).
///
/// [2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
pub const CLIENT_TO_SERVER_CHANNEL_NAME: &str = "TextInput_ClientToServerDVC";

mod wire;

pub mod pdu;

#[cfg(feature = "std")]
pub mod client;
#[cfg(feature = "std")]
pub mod server;
