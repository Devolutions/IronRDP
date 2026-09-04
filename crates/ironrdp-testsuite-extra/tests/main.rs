#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary
#![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]
#![allow(
    dead_code,
    unreachable_pub,
    reason = "Mstsgu integration tests compile private HTTP and RPC source modules"
)]

macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{ <$crate::Error as $crate::GwErrorExt>::custom($context, $source) }};
}

mod agent;
mod async_framed;
mod capture_helpers;
mod client;
mod dvc_pipe_proxy;
mod e2e;
mod gateway_detect;
mod mstsgu;
pub(crate) use mstsgu::rpch_http::{Error, GwErrorExt, GwErrorKind};
#[path = "../../ironrdp-mstsgu/src/http_auth.rs"]
#[expect(
    unexpected_cfgs,
    reason = "the included Mstsgu source owns the smartcard configuration"
)]
mod http_auth;
#[path = "../../ironrdp-mstsgu/src/mock_rpch.rs"]
mod mock_rpch;
mod rdpeudp_tokio;
#[path = "../../ironrdp-mstsgu/src/rpc.rs"]
mod rpc;
#[path = "../../ironrdp-mstsgu/src/rpc_transport.rs"]
mod rpc_transport;
mod vmconnect;
mod volume;

pub(crate) use ironrdp_mstsgu::GwSmartCardCredentials;
