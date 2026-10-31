#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

/// Dynamic virtual channel name from [MS-RDPET 2.1].
///
/// [MS-RDPET 2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpet/245d4ee3-df26-40a3-85b7-c6ffacfef386
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Telemetry";

pub mod client;
pub mod pdu;
