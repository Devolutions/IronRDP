#![doc = include_str!("../README.md")]

#[macro_use]
extern crate tracing;

pub mod renderer;

mod draw;
mod surface;

type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
type Result<T> = std::result::Result<T, Error>;
