#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary

// No need to be as strict as in production libraries
#![allow(clippy::arithmetic_side_effects)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

#[macro_use]
extern crate tracing;

pub mod app;
pub mod clipboard;
pub mod config;
pub mod network_client;
pub mod rdp;
