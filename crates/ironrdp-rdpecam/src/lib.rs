#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

/// Device enumeration channel name from [2.1] Transport.
///
/// [2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/
pub const ENUMERATION_CHANNEL_NAME: &str = "RDCamera_Device_Enumerator";

pub mod client;
pub mod pdu;

pub use client::{
    CameraBackend, CameraRedirectionSession, DeviceChannelListener, DeviceClient, EnumerationClient, RedirectedDevice,
};
pub use pdu::{
    DeviceDescriptor, ErrorCode, MediaFormat, MediaType, ProtocolVersion, StartStreamInfo, StreamDescription,
};
