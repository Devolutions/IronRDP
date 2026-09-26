#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

/// Dynamic virtual channel listener name from [MS-RDPEL] section 2.1.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Location";

pub mod client;
pub mod pdu;
