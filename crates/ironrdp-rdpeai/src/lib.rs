#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

/// Dynamic virtual channel listener name per MS-RDPEAI §2.1.
pub const CHANNEL_NAME: &str = "AUDIO_INPUT";

pub mod client;
pub mod pdu;

pub use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
