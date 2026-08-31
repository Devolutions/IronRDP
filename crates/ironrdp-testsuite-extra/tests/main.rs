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
#[path = "../../ironrdp-mstsgu/src/http_auth.rs"]
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

#[derive(Clone)]
pub(crate) struct GwSmartCardCredentials {
    #[cfg(feature = "smartcard")]
    pub(crate) username: String,
    #[cfg(feature = "smartcard")]
    pub(crate) pin: String,
    #[cfg(feature = "smartcard")]
    pub(crate) certificate: Vec<u8>,
    #[cfg(feature = "smartcard")]
    pub(crate) private_key: Option<Vec<u8>>,
    #[cfg(feature = "smartcard")]
    pub(crate) reader_name: String,
    #[cfg(feature = "smartcard")]
    pub(crate) card_name: Option<String>,
    #[cfg(feature = "smartcard")]
    pub(crate) container_name: Option<String>,
    #[cfg(feature = "smartcard")]
    pub(crate) csp_name: Option<String>,
}
