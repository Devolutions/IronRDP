#[cfg(feature = "native-tls")]
mod consent;
#[cfg(feature = "native-tls")]
mod http_auth;
#[cfg(feature = "native-tls")]
mod http_control;
#[cfg(feature = "native-tls")]
mod packet_io;
#[cfg(feature = "rustls")]
mod packet_io_rustls;
#[expect(dead_code, unreachable_pub, reason = "tests import private protocol structures")]
#[path = "../../../ironrdp-mstsgu/src/proto.rs"]
#[expect(
    clippy::allow_attributes,
    reason = "the imported protocol source contains an intentionally unfulfilled expectation"
)]
#[allow(unfulfilled_lint_expectations)]
mod proto;
#[cfg(feature = "native-tls")]
mod proxy;
mod rpc_pdu;
mod rpc_tsgu_stubs;
pub(crate) mod rpch_http;
mod rpch_v2;
mod tunnel_policy;
mod udp;
