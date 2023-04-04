//! Rust implementation of the Remote Desktop Protocol (RDP).
//!
//! This is a meta crate re-exporting other ironrdp crates for convenience.

#[cfg(feature = "pdu")]
pub use ironrdp_pdu as pdu;

#[cfg(feature = "connector")]
pub use ironrdp_connector as connector;

#[cfg(feature = "session")]
pub use ironrdp_session as session;

#[cfg(feature = "graphics")]
pub use ironrdp_graphics as graphics;

#[cfg(feature = "input")]
pub use ironrdp_input as input;

#[cfg(feature = "tokio")]
pub use ironrdp_async as tokio;

#[cfg(feature = "futures")]
pub use ironrdp_async as futures;
