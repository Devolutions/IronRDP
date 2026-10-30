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

/// A PDU could not be queued because the queue for the peer is full.
///
/// The replies queued while one message is processed are required by the peer, so the
/// processors fail the channel rather than drop them.
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
