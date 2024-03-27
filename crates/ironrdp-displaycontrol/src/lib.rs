#![doc = include_str!("../README.md")]

pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

pub mod client;
pub mod pdu;
pub mod server;
