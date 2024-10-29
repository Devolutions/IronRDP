#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://webdevolutions.blob.core.windows.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg"
)]
#![allow(clippy::arithmetic_side_effects)] // TODO: should we enable this lint back?

pub use tokio;
pub use tokio_rustls;

#[macro_use]
extern crate tracing;

mod builder;
mod capabilities;
mod clipboard;
mod display;
mod encoder;
mod handler;
mod server;
mod sound;

pub use clipboard::*;
pub use display::*;
pub use handler::*;
pub use server::*;
pub use sound::*;
