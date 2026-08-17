#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]

//! RAIL static-channel wire types.

#[cfg(feature = "alloc")]
extern crate alloc;

/// The name of the Remote Applications Integrated Locally static channel.
pub const CHANNEL_NAME: &str = "RAIL";

#[cfg(feature = "alloc")]
pub mod pdu;

#[cfg(feature = "alloc")]
pub use pdu::*;
