#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

/// ECHO dynamic virtual channel name per MS-RDPEECO.
pub const CHANNEL_NAME: &str = "ECHO";

pub mod client;
pub mod pdu;
pub mod server;
