//! Shared local RPC protocol and transport used by IronRDP local clients.

pub mod ipc;
pub mod transport;
#[cfg(feature = "__test")]
pub mod wire;
#[cfg(not(feature = "__test"))]
#[expect(
    unreachable_pub,
    reason = "the __test feature exposes this module to the shared integration tests"
)]
pub(crate) mod wire;
