#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[macro_use]
extern crate tracing;

mod error;
mod message;
mod os_pipe;
mod platform;
mod proxy;
mod worker;

pub use self::proxy::DvcNamedPipeProxy;
