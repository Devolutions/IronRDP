//! DVC COM client plugin loader for IronRDP (Windows-only).
//!
//! This crate enables loading native Windows DVC (Dynamic Virtual Channel) client plugin DLLs
//! such as `webauthn.dll` into IronRDP's DVC channel infrastructure.
//!
//! The plugin DLL is loaded via `LoadLibraryW`, its `VirtualChannelGetInstance` export is called
//! to obtain `IWTSPlugin` COM objects, and a Rust implementation of `IWTSVirtualChannelManager`
//! bridges data bidirectionally between the plugin's COM callbacks and IronRDP's DVC system.
//!
//! # Architecture
//!
//! A dedicated COM worker thread owns all COM objects (which are `!Send`). The [`DvcComChannel`]
//! structs (which implement `DvcProcessor + Send`) are registered as DVC channels in IronRDP's
//! `DrdynvcClient` and communicate with the COM thread via `std::sync::mpsc` channels.
//!
//! Outbound data from the plugin (`IWTSVirtualChannel::Write`) is injected into the active
//! session loop via the `on_write_dvc` callback, following the same pattern as
//! `ironrdp-dvc-pipe-proxy`.
//!
//! # References
//!
//! - [Writing a Client DVC Component](https://learn.microsoft.com/en-us/windows/win32/termserv/writing-a-client-dvc-component)
//! - [tsvirtualchannels.h](https://learn.microsoft.com/en-us/windows/win32/api/tsvirtualchannels/)

#![cfg(windows)]
// The `windows` crate's `#[implement]` macro generates code that triggers these lints.
// We must allow them crate-wide since the generated code is not under our control.
#![allow(clippy::inline_always)]
#![allow(clippy::as_pointer_underscore)]
#![allow(clippy::multiple_unsafe_ops_per_block)]
#![allow(clippy::undocumented_unsafe_blocks)]
#![allow(clippy::unnecessary_safety_comment)]

mod channel;
mod com;
mod worker;

pub use channel::{load_dvc_plugin, DvcComChannel};
