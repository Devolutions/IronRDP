//! Hyper-V vmconnect support for IronRDP.
//!
//! A vmconnect session (Hyper-V host port 2179) reorders the RDP connection sequence: the
//! client first sends a preconnection blob carrying the VM ID, then performs TLS + CredSSP
//! against the *host*, and only then runs X.224 negotiation. [`VmClientConnector`] drives that
//! pre-connection sequence and hands the session back to the standard
//! [`ClientConnector`](ironrdp_connector::ClientConnector) for the rest of the connection.

mod connector;

pub use connector::{VmClientConnector, run_until_handover};
