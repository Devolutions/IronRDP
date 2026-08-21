//! Error types for the async UDP transport.

use core::fmt;
use std::io;

use ironrdp_pdu::rdp::multitransport::RequestedProtocol;
use ironrdp_rdpemt::RdpemtError;
use ironrdp_rdpeudp::RdpeudpError;

pub type DriverResult<T> = Result<T, DriverError>;

pub type DriverError = ironrdp_error::Error<DriverErrorKind>;

/// A fatal condition in the driver task.
#[non_exhaustive]
#[derive(Debug)]
pub enum DriverErrorKind {
    /// The UDP socket failed.
    Socket(io::Error),

    /// The RDP-UDP state machine rejected an operation.
    Rdpeudp(RdpeudpError),

    /// The peer closed the connection, or the idle timeout fired.
    ConnectionClosed,
}

impl fmt::Display for DriverErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Socket(_) => write!(f, "UDP socket error"),
            Self::Rdpeudp(_) => write!(f, "RDP-UDP error"),
            Self::ConnectionClosed => write!(f, "connection is closed"),
        }
    }
}

impl core::error::Error for DriverErrorKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Socket(error) => Some(error),
            Self::Rdpeudp(error) => Some(error),
            Self::ConnectionClosed => None,
        }
    }
}

pub trait DriverErrorExt {
    fn socket(context: &'static str, error: io::Error) -> Self;
    fn rdpeudp(context: &'static str, error: RdpeudpError) -> Self;
    fn connection_closed(context: &'static str) -> Self;
}

impl DriverErrorExt for DriverError {
    #[track_caller]
    fn socket(context: &'static str, error: io::Error) -> Self {
        Self::new(context, DriverErrorKind::Socket(error))
    }

    #[track_caller]
    fn rdpeudp(context: &'static str, error: RdpeudpError) -> Self {
        Self::new(context, DriverErrorKind::Rdpeudp(error))
    }

    #[track_caller]
    fn connection_closed(context: &'static str) -> Self {
        Self::new(context, DriverErrorKind::ConnectionClosed)
    }
}

pub type UdpTransportResult<T> = Result<T, UdpTransportError>;

pub type UdpTransportError = ironrdp_error::Error<UdpTransportErrorKind>;

/// A failure somewhere in the connection sequence: UDP handshake, then TLS,
/// then the multitransport tunnel.
#[non_exhaustive]
#[derive(Debug)]
pub enum UdpTransportErrorKind {
    /// The UDP socket failed.
    Socket(io::Error),

    /// The RDP-UDP handshake failed.
    Handshake(DriverError),

    /// The RDP-UDP handshake did not complete within the configured budget.
    HandshakeTimeout,

    /// The TLS handshake failed.
    Tls(io::Error),

    /// The tunnel state machine rejected an operation.
    Rdpemt(RdpemtError),

    /// The peer rejected the tunnel creation request.
    TunnelRejected { hr_response: u32 },

    /// The driver task ended without reporting an outcome.
    DriverPanic,

    /// The driver task reported a fatal condition.
    Driver(DriverError),

    /// The server requested a transport variant this driver does not
    /// implement (only `UdpFecR` is supported; `UdpFecL` is not).
    UnsupportedProtocol { requested: RequestedProtocol },

    /// A `send()` payload exceeds the wire `PayloadLength` field's 65535-byte
    /// capacity ([MS-RDPEMT] 2.2.2.3, `RDP_TUNNEL_DATA`).
    PayloadTooLarge { len: usize },
}

impl fmt::Display for UdpTransportErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Socket(_) => write!(f, "UDP socket error"),
            Self::Handshake(_) => write!(f, "RDP-UDP handshake failed"),
            Self::HandshakeTimeout => write!(f, "RDP-UDP handshake timed out"),
            Self::Tls(_) => write!(f, "TLS error"),
            Self::Rdpemt(_) => write!(f, "multitransport tunnel error"),
            Self::TunnelRejected { hr_response } => {
                write!(f, "tunnel creation rejected: HRESULT {hr_response:#010x}")
            }
            Self::DriverPanic => write!(f, "driver task ended without an outcome"),
            Self::Driver(_) => write!(f, "driver error"),
            Self::UnsupportedProtocol { requested } => {
                write!(f, "requested transport protocol not supported: {requested:?}")
            }
            Self::PayloadTooLarge { len } => {
                write!(
                    f,
                    "send payload of {len} bytes exceeds the 65535-byte tunnel data limit"
                )
            }
        }
    }
}

impl core::error::Error for UdpTransportErrorKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Socket(error) => Some(error),
            Self::Tls(error) => Some(error),
            Self::Handshake(error) => Some(error),
            Self::Driver(error) => Some(error),
            Self::Rdpemt(error) => Some(error),
            Self::HandshakeTimeout | Self::TunnelRejected { .. } | Self::DriverPanic => None,
            Self::UnsupportedProtocol { .. } | Self::PayloadTooLarge { .. } => None,
        }
    }
}

pub trait UdpTransportErrorExt {
    fn socket(context: &'static str, error: io::Error) -> Self;
    fn handshake(context: &'static str, error: DriverError) -> Self;
    fn handshake_timeout(context: &'static str) -> Self;
    fn tls(context: &'static str, error: io::Error) -> Self;
    fn rdpemt(context: &'static str, error: RdpemtError) -> Self;
    fn tunnel_rejected(context: &'static str, hr_response: u32) -> Self;
    fn driver_panic(context: &'static str) -> Self;
    fn driver(context: &'static str, error: DriverError) -> Self;
    fn unsupported_protocol(context: &'static str, requested: RequestedProtocol) -> Self;
    fn payload_too_large(context: &'static str, len: usize) -> Self;
}

impl UdpTransportErrorExt for UdpTransportError {
    #[track_caller]
    fn socket(context: &'static str, error: io::Error) -> Self {
        Self::new(context, UdpTransportErrorKind::Socket(error))
    }

    #[track_caller]
    fn handshake(context: &'static str, error: DriverError) -> Self {
        Self::new(context, UdpTransportErrorKind::Handshake(error))
    }

    #[track_caller]
    fn handshake_timeout(context: &'static str) -> Self {
        Self::new(context, UdpTransportErrorKind::HandshakeTimeout)
    }

    #[track_caller]
    fn tls(context: &'static str, error: io::Error) -> Self {
        Self::new(context, UdpTransportErrorKind::Tls(error))
    }

    #[track_caller]
    fn rdpemt(context: &'static str, error: RdpemtError) -> Self {
        Self::new(context, UdpTransportErrorKind::Rdpemt(error))
    }

    #[track_caller]
    fn tunnel_rejected(context: &'static str, hr_response: u32) -> Self {
        Self::new(context, UdpTransportErrorKind::TunnelRejected { hr_response })
    }

    #[track_caller]
    fn driver_panic(context: &'static str) -> Self {
        Self::new(context, UdpTransportErrorKind::DriverPanic)
    }

    #[track_caller]
    fn driver(context: &'static str, error: DriverError) -> Self {
        Self::new(context, UdpTransportErrorKind::Driver(error))
    }

    #[track_caller]
    fn unsupported_protocol(context: &'static str, requested: RequestedProtocol) -> Self {
        Self::new(context, UdpTransportErrorKind::UnsupportedProtocol { requested })
    }

    #[track_caller]
    fn payload_too_large(context: &'static str, len: usize) -> Self {
        Self::new(context, UdpTransportErrorKind::PayloadTooLarge { len })
    }
}
