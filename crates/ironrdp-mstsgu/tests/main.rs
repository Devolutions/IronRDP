#![allow(unused_crate_dependencies)]

//! Integration tests are organized in one binary to avoid repeatedly relinking the library.

#[cfg(feature = "native-tls")]
mod http_auth;
mod http_control;
mod rpc_pdu;
mod udp;
