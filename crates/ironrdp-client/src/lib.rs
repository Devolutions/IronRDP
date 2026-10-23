#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
// No need to be as strict as in production libraries
#![allow(clippy::arithmetic_side_effects)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

#[cfg(feature = "sound")]
pub mod audio;
#[cfg(feature = "camera")]
pub mod camera;
pub mod config;
pub mod egfx_cache;
pub mod h264;
pub mod output_channel;
pub mod rail;
pub mod rdp;
#[cfg(feature = "video")]
pub mod video;

/// USB device redirection backend (MS-RDPEUSB), re-exported for device selection.
#[cfg(feature = "usb")]
pub use ironrdp_rdpeusb_native as usb;

#[cfg(all(any(windows, target_os = "linux"), feature = "clipboard"))]
mod clipboard;
mod redirection;

#[cfg(feature = "usb")]
mod urbdrc;

mod ws;
