#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

pub mod client;
pub mod pdu;
pub mod server;
