//! Rust implementation of the Remote Desktop Protocol (RDP).
//!
//! This is a meta crate re-exporting other ironrdp crates for convenience.

#[cfg(feature = "acceptor")]
pub use ironrdp_acceptor as acceptor;
#[cfg(feature = "cliprdr")]
pub use ironrdp_cliprdr as cliprdr;
#[cfg(feature = "connector")]
pub use ironrdp_connector as connector;
#[cfg(feature = "dvc")]
pub use ironrdp_dvc as dvc;
#[cfg(feature = "graphics")]
pub use ironrdp_graphics as graphics;
#[cfg(feature = "input")]
pub use ironrdp_input as input;
#[cfg(feature = "pdu")]
pub use ironrdp_pdu as pdu;
#[cfg(feature = "rdpdr")]
pub use ironrdp_rdpdr as rdpdr;
#[cfg(feature = "rdpsnd")]
pub use ironrdp_rdpsnd as rdpsnd;
#[cfg(feature = "server")]
pub use ironrdp_server as server;
#[cfg(feature = "session")]
pub use ironrdp_session as session;
#[cfg(feature = "svc")]
pub use ironrdp_svc as svc;
