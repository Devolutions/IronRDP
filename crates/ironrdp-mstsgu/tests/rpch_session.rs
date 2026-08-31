#![allow(dead_code, unreachable_pub, unused_crate_dependencies)]

use core::fmt;

type Error = ironrdp_error::Error<GwErrorKind>;

#[derive(Debug)]
enum GwErrorKind {
    Connect,
    GatewayCode(u32),
    HttpStatus(u16),
    PacketEof,
    Custom,
    Encode,
    Decode,
}

trait GwErrorExt {
    fn custom<E>(context: &'static str, error: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static;
}

impl GwErrorExt for Error {
    fn custom<E>(context: &'static str, error: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static,
    {
        Self::new(context, GwErrorKind::Custom).with_source(error)
    }
}

impl fmt::Display for GwErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect => f.write_str("connection error"),
            Self::GatewayCode(code) => write!(f, "gateway error 0x{code:08x}"),
            Self::HttpStatus(status) => write!(f, "unexpected http status {status}"),
            Self::PacketEof => f.write_str("packet eof"),
            Self::Custom => f.write_str("custom"),
            Self::Encode => f.write_str("encode"),
            Self::Decode => f.write_str("decode"),
        }
    }
}

impl core::error::Error for GwErrorKind {}

macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{ <$crate::Error as $crate::GwErrorExt>::custom($context, $source) }};
}

#[path = "../src/mock_rpch.rs"]
mod mock_rpch;
#[path = "../src/rpc.rs"]
mod rpc;
#[path = "../src/rpc_transport.rs"]
mod rpc_transport;
#[path = "../src/rpch.rs"]
mod rpch;
