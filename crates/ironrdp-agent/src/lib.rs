#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

//! A CLI-driven, daemon-backed agentic RDP client.
//!
//! The public surface is intentionally small and split into:
//!
//! - [`ipc`]: the strictly-typed request/response schema and its binary codec.
//! - [`transport`]: the local IPC transport (Unix socket / Windows named pipe) and framing.
//! - [`daemon`]: the long-lived daemon driver.
//! - [`cli`]: the short-lived CLI driver.

pub mod cli;
pub mod daemon;
pub mod ipc;
pub mod transport;

pub(crate) mod help;
pub(crate) mod logbuf;

// The wire codec helpers are internal, but the `internal` feature exposes them (hidden from docs)
// so they can be unit tested from the workspace test suite.
#[cfg(feature = "internal")]
#[doc(hidden)]
pub mod wire;
#[cfg(not(feature = "internal"))]
pub(crate) mod wire;
