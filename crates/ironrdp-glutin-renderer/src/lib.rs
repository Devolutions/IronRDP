#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg"
)]

#[macro_use]
extern crate tracing;

pub mod renderer;

mod draw;
mod surface;

type Error = Box<dyn core::error::Error + Send + Sync + 'static>;
type Result<T> = std::result::Result<T, Error>;
