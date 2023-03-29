//! Rust implementation of the Remote Desktop Protocol (RDP).
//!
//! This is a meta crate re-exporting ironrdp-pdu, ironrdp-session, ironrdp-graphics
//! and ironrdp-input crates for convenience.

pub use {ironrdp_graphics as graphics, ironrdp_input as input, ironrdp_pdu as pdu, ironrdp_session as session};
