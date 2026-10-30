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
mod channel;
#[cfg(feature = "std")]
pub mod client;
#[cfg(feature = "std")]
pub mod server;

/// Largest reassembled message accepted on the receiving channel.
///
/// Text paging bounds each text update to a few tens of thousands of characters
/// ([1.3.3] Text Paging).
///
/// [1.3.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[cfg(feature = "std")]
pub const MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

/// Largest number of PDUs queued for the peer, while the sending channel is closed or until the
/// session sends them.
#[cfg(feature = "std")]
pub const MAX_PENDING_PDUS: usize = 1024;

/// A PDU could not be queued because [`MAX_PENDING_PDUS`] PDUs are queued for the peer already.
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

#[cfg(feature = "std")]
impl core::fmt::Display for QueueFull {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("too many text input PDUs are queued")
    }
}

#[cfg(feature = "std")]
impl core::error::Error for QueueFull {}
