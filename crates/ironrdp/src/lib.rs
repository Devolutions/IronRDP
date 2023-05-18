//! Rust implementation of the Remote Desktop Protocol (RDP).
//!
//! This is a meta crate re-exporting other ironrdp crates for convenience.

#[cfg(feature = "connector")]
pub use ironrdp_connector as connector;
#[cfg(feature = "graphics")]
pub use ironrdp_graphics as graphics;
#[cfg(feature = "input")]
pub use ironrdp_input as input;
#[cfg(feature = "pdu")]
pub use ironrdp_pdu as pdu;
#[cfg(feature = "session")]
pub use ironrdp_session as session;
#[cfg(feature = "tls")]
pub use ironrdp_tls as tls;
#[cfg(feature = "tokio")]
pub use ironrdp_tokio as tokio;
