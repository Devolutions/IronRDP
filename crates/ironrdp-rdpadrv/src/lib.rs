#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

/// Dynamic virtual channel name for audio levels, from [MS-RDPADRV] 2.1 "Transport".
///
/// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
pub const AUDIO_LEVEL_CHANNEL_NAME: &str = "WMSAud";

/// Dynamic virtual channel name for drive letters, from [MS-RDPADRV] 2.1 "Transport".
///
/// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
pub const DRIVE_LETTER_CHANNEL_NAME: &str = "WMSDL";

pub mod client;
pub mod pdu;
pub mod server;
