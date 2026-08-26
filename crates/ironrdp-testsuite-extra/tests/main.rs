#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary
#![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]

mod agent;
mod async_framed;
mod capture_helpers;
mod client;
mod dvc_pipe_proxy;
mod e2e;
mod gateway_detect;
mod rdpeudp_tokio;
mod viewer;
mod vmconnect;
mod volume;
