#![allow(unreachable_pub, unused_crate_dependencies)]

use core::fmt;

type Error = ironrdp_error::Error<GwErrorKind>;

#[derive(Debug)]
enum GwErrorKind {
    InvalidCertificateValidation,
    Connect,
    GatewayCode(u32),
    HttpStatus(u16),
    PacketEof,
    UnsupportedFeature,
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
            Self::InvalidCertificateValidation => f.write_str("invalid certificate validation configuration"),
            Self::Connect => f.write_str("connection error"),
            Self::GatewayCode(code) => write!(f, "gateway error 0x{code:08x}"),
            Self::HttpStatus(status) => write!(f, "unexpected http status {status}"),
            Self::PacketEof => f.write_str("packet eof"),
            Self::UnsupportedFeature => f.write_str("unsupported feature"),
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

#[derive(Clone)]
struct GwSmartCardCredentials;

struct GwConnectTarget {
    gw_endpoint: String,
    gw_user: String,
    gw_pass: String,
    smart_card: Option<GwSmartCardCredentials>,
    server: String,
}

#[derive(Clone, Copy, Default)]
#[expect(
    dead_code,
    reason = "the integration harness does not inspect authorization policy flags"
)]
struct GwRpchRedirectionFlags {
    enable_all: bool,
    disable_all: bool,
    drive_disabled: bool,
    printer_disabled: bool,
    port_disabled: bool,
    clipboard_disabled: bool,
    pnp_disabled: bool,
}

#[path = "../src/http_auth.rs"]
#[expect(
    dead_code,
    reason = "the integration harness only exercises shared HTTP authentication through RPCH"
)]
mod http_auth;
#[path = "../src/mock_rpch.rs"]
mod mock_rpch;
#[path = "../src/rpc.rs"]
#[expect(
    dead_code,
    reason = "the integration harness exercises only the RPCH session codec subset"
)]
mod rpc;
#[path = "../src/rpc_transport.rs"]
mod rpc_transport;
mod packet_io {
    use tokio::io::{AsyncRead, AsyncWrite};

    pub(crate) trait GatewayIo: AsyncRead + AsyncWrite + Unpin {}

    impl<T: AsyncRead + AsyncWrite + Unpin + ?Sized> GatewayIo for T {}

    pub(crate) type GatewayStream = Box<dyn GatewayIo + Send + 'static>;

    pub(crate) async fn open_rpch_gateway_channels(
        _endpoint: &str,
        _certificate_validation: ironrdp_tls::CertificateValidation,
        _certificate_validation_callback: Option<ironrdp_tls::CertificateValidationCallback>,
    ) -> Result<(GatewayStream, GatewayStream, core::net::SocketAddr), crate::Error> {
        Err(crate::Error::new(
            "unexpected network RPCH connection",
            crate::GwErrorKind::Connect,
        ))
    }

    pub(crate) async fn open_rpch_gateway_stream(
        _endpoint: &str,
        _certificate_validation: ironrdp_tls::CertificateValidation,
        _certificate_validation_callback: Option<ironrdp_tls::CertificateValidationCallback>,
    ) -> Result<(GatewayStream, core::net::SocketAddr), crate::Error> {
        Err(crate::Error::new(
            "unexpected network RPCH connection",
            crate::GwErrorKind::Connect,
        ))
    }
}
#[path = "../src/rpch.rs"]
#[expect(
    dead_code,
    reason = "the live raw channel is exercised by the rpch_client integration target"
)]
mod rpch;
