#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub const CHANNEL_NAME: &str = "URBDRC";

pub mod client;
pub mod pdu;
