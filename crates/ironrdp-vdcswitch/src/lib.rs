#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

/// Dynamic virtual channel name Windows 365 Cloud PCs open for Boot and Switch mode.
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::VirtualDesktopControl::Switch";

pub mod client;
pub mod pdu;
