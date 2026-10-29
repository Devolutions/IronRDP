#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

/// Control channel name from [2.1] Transport.
///
/// [2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
pub const CONTROL_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Video::Control::v08.01";
/// Data channel name from [2.1] Transport.
///
/// [2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
pub const DATA_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Video::Data::v08.01";

pub mod client;
pub mod pdu;
pub mod presenter;

pub use client::{SharedPresenter, VideoControlClient, VideoDataClient};
pub use presenter::{Clock, DecoderFactory, VideoPresenter, VideoUpdate};
