#![cfg_attr(doc, doc = include_str!("../README.md"))]
// `image`, `ironrdp-client`, `tracing-subscriber` are referenced from `main.rs` only.
#![expect(unused_crate_dependencies, reason = "consumed by the binary target")]

pub mod cli;
pub mod descriptions;
pub mod help;
pub mod ipc;
pub mod redact;
