#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary
#![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]

#[cfg(feature = "full")]
mod agent;
mod async_framed;
mod capture_helpers;
#[cfg(feature = "full")]
mod client_config;
mod dvc_pipe_proxy;
#[cfg(feature = "rustls")]
mod e2e;
mod gateway_detect;
#[cfg(feature = "native-tls")]
mod mstsgu_http_auth;
mod mstsgu_http_control;
mod mstsgu_rpc_pdu;
mod mstsgu_udp;
mod rdpeudp_tokio;
mod vmconnect;
mod volume;
