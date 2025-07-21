#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[cfg(test)]
use tracing_subscriber as _;

#[macro_use]
extern crate tracing;

pub mod cpal;
