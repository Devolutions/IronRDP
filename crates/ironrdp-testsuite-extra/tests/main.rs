#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary
#![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]

mod agent;
mod capture_helpers;
mod client_config;
mod dvc_pipe_proxy;
mod e2e;
mod rdpeudp_tokio;
mod vmconnect;
mod volume;
