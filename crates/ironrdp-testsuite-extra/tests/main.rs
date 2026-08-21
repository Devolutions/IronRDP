#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary
#![allow(clippy::panic, reason = "panic is fine in tests")]
#![allow(clippy::std_instead_of_core, reason = "std is fine in integration tests")]
#![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]

#[cfg(feature = "full")]
mod agent;
mod async_framed;
mod capture_helpers;
#[cfg(feature = "full")]
mod client;
mod dvc_pipe_proxy;
#[cfg(feature = "rustls")]
mod e2e;
mod gateway_detect;
mod mstsgu;
pub(crate) use mstsgu::rpch_http::{Error, GwErrorExt, GwErrorKind};
mod rdpeudp_tokio;
mod vmconnect;
mod volume;
