#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[macro_use]
extern crate tracing;

#[cfg(target_os = "windows")]
mod windows;

mod platform;
pub use self::platform::DvcNamedPipeProxy;
