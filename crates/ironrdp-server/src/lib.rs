#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![allow(clippy::arithmetic_side_effects)] // TODO: should we enable this lint back?

pub use {tokio, tokio_rustls};

#[macro_use]
extern crate tracing;

#[macro_use]
mod macros;

mod builder;
mod capabilities;
mod clipboard;
mod display;
mod encoder;
mod handler;
#[cfg(feature = "helper")]
mod helper;
mod server;
mod sound;

pub use clipboard::*;
pub use display::*;
pub use handler::*;
#[cfg(feature = "helper")]
pub use helper::*;
pub use server::*;
pub use sound::*;

#[cfg(feature = "__bench")]
pub mod bench {
    pub mod encoder {
        pub mod rfx {
            pub use crate::encoder::rfx::bench::{rfx_enc, rfx_enc_tile};
        }

        pub use crate::encoder::{UpdateEncoder, UpdateEncoderCodecs};
    }
}
