#![allow(dead_code, unreachable_pub, unused_crate_dependencies)]

use core::fmt;

type Error = ironrdp_error::Error<GwErrorKind>;

#[derive(Debug)]
enum GwErrorKind {
    Connect,
    Custom,
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
        f.write_str(match self {
            Self::Connect => "connection error",
            Self::Custom => "custom",
        })
    }
}

impl core::error::Error for GwErrorKind {}

#[path = "../src/rpc.rs"]
mod rpc;
