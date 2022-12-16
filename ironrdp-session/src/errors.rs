use std::{
    io,
    sync::{
        mpsc::{RecvError, SendError},
        PoisonError,
    },
};

use failure::Fail;
use ironrdp::{
    codecs,
    dvc::{display, gfx},
    fast_path::FastPathError,
    nego,
    rdp::{self, server_license::ServerLicenseError},
    McsError,
};

#[derive(Debug, Fail)]
pub enum RdpError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "connection error: {}", _0)]
    ConnectionError(#[fail(cause)] io::Error),
    #[fail(display = "X.224 error: {}", _0)]
    X224Error(#[fail(cause)] io::Error),
    #[fail(display = "negotiation error: {}", _0)]
    NegotiationError(#[fail(cause)] nego::NegotiationError),
    #[fail(display = "unexpected PDU: {}", _0)]
    UnexpectedPdu(String),
    #[fail(display = "Unexpected disconnection: {}", _0)]
    UnexpectedDisconnection(String),
    #[fail(display = "invalid response: {}", _0)]
    InvalidResponse(String),
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    #[fail(display = "TLS connector error: {}", _0)]
    TlsConnectorError(native_tls::Error),
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    #[fail(display = "TLS handshake error: {}", _0)]
    TlsHandshakeError(native_tls::Error),
    #[cfg(feature = "rustls")]
    #[fail(display = "TLS connector error: {}", _0)]
    TlsConnectorError(rustls::Error),
    #[cfg(feature = "rustls")]
    #[fail(display = "TLS handshake error: {}", _0)]
    TlsHandshakeError(rustls::Error),
    #[fail(display = "CredSSP error: {}", _0)]
    CredSspError(#[fail(cause)] sspi::Error),
    #[fail(display = "CredSSP TSRequest error: {}", _0)]
    TsRequestError(#[fail(cause)] io::Error),
    #[fail(display = "early User Authentication Result error: {}", _0)]
    EarlyUserAuthResultError(#[fail(cause)] io::Error),
    #[fail(display = "the server denied access via Early User Authentication Result")]
    AccessDenied,
    #[fail(display = "MCS Connect error: {}", _0)]
    McsConnectError(#[fail(cause)] McsError),
    #[fail(display = "failed to get info about the user: {}", _0)]
    UserInfoError(String),
    #[fail(display = "MCS error: {}", _0)]
    McsError(McsError),
    #[fail(display = "Client Info PDU error: {}", _0)]
    ClientInfoError(rdp::RdpError),
    #[fail(display = "Server License PDU error: {}", _0)]
    ServerLicenseError(rdp::RdpError),
    #[fail(display = "Share Control Header error: {}", _0)]
    ShareControlHeaderError(rdp::RdpError),
    #[fail(display = "capability sets error: {}", _0)]
    CapabilitySetsError(rdp::RdpError),
    #[fail(display = "Virtual channel error: {}", _0)]
    VirtualChannelError(rdp::vc::ChannelError),
    #[fail(display = "Invalid channel id error: {}", _0)]
    InvalidChannelIdError(String),
    #[fail(display = "Graphics pipeline protocol error: {}", _0)]
    GraphicsPipelineError(gfx::GraphicsPipelineError),
    #[fail(display = "Display pipeline protocol error: {}", _0)]
    DisplayPipelineError(display::DisplayPipelineError),
    #[fail(display = "ZGFX error: {}", _0)]
    ZgfxError(#[fail(cause)] gfx::zgfx::ZgfxError),
    #[fail(display = "Fast-Path error: {}", _0)]
    FastPathError(#[fail(cause)] FastPathError),
    #[fail(display = "RDP error: {}", _0)]
    RdpError(#[fail(cause)] ironrdp::RdpError),
    #[fail(display = "access to the non-existing channel: {}", _0)]
    AccessToNonExistingChannel(u32),
    #[fail(display = "access to the non-existing channel name: {}", _0)]
    AccessToNonExistingChannelName(String),
    #[fail(display = "data in unexpected channel: {}", _0)]
    UnexpectedChannel(u16),
    #[fail(display = "unexpected Surface Command codec ID: {}", _0)]
    UnexpectedCodecId(u8),
    #[fail(display = "RDP error: {}", _0)]
    RfxError(#[fail(cause)] codecs::rfx::RfxError),
    #[fail(display = "absence of mandatory Fast-Path header")]
    MandatoryHeaderIsAbsent,
    #[fail(display = "RLGR error: {}", _0)]
    RlgrError(#[fail(cause)] codecs::rfx::rlgr::RlgrError),
    #[fail(display = "absence of RFX channels")]
    NoRfxChannelsAnnounced,
    #[fail(
        display = "the server that started working using the inconsistent protocol: {:?}",
        _0
    )]
    UnexpectedFastPathUpdate(ironrdp::fast_path::UpdateCode),
    #[fail(display = "server error: {}", _0)]
    ServerError(String),
    #[fail(display = "Missing peer certificate")]
    MissingPeerCertificate,
    #[fail(display = "Dynamic virtual channel not connected")]
    DynamicVirtualChannelNotConnected,
    #[fail(display = "Static global channel not connected")]
    StaticChannelNotConnected,
    #[fail(display = "Invalid Capabilities mask provided. Mask: {:X}", _0)]
    InvalidCapabilitiesMask(u32),
    #[fail(display = "Stream terminated while waiting for some data")]
    UnexpectedStreamTermination,
    #[fail(display = "Unable to send message on channel {}", _0)]
    SendError(String),
    #[fail(display = "Unable to recieve message on channel {}", _0)]
    RecieveError(String),
    #[fail(display = "Lock poisoned")]
    LockPoisonedError,
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    #[fail(display = "Invalid DER structure: {}", _0)]
    DerEncode(#[fail(cause)] native_tls::Error),
}

impl From<io::Error> for RdpError {
    fn from(e: io::Error) -> Self {
        RdpError::IOError(e)
    }
}

#[cfg(feature = "rustls")]
impl From<rustls::Error> for RdpError {
    fn from(e: rustls::Error) -> Self {
        match e {
            rustls::Error::InappropriateHandshakeMessage { .. } | rustls::Error::HandshakeNotComplete => {
                RdpError::TlsHandshakeError(e)
            }
            _ => RdpError::TlsConnectorError(e),
        }
    }
}

impl<T> From<SendError<T>> for RdpError {
    fn from(e: SendError<T>) -> Self {
        RdpError::SendError(e.to_string())
    }
}

impl From<RecvError> for RdpError {
    fn from(e: RecvError) -> Self {
        RdpError::RecieveError(e.to_string())
    }
}

impl<T> From<PoisonError<T>> for RdpError {
    fn from(_e: PoisonError<T>) -> Self {
        RdpError::LockPoisonedError
    }
}

impl From<nego::NegotiationError> for RdpError {
    fn from(e: nego::NegotiationError) -> Self {
        RdpError::NegotiationError(e)
    }
}

impl From<McsError> for RdpError {
    fn from(e: McsError) -> Self {
        RdpError::McsError(e)
    }
}

impl From<rdp::vc::ChannelError> for RdpError {
    fn from(e: rdp::vc::ChannelError) -> Self {
        RdpError::VirtualChannelError(e)
    }
}

impl From<gfx::GraphicsPipelineError> for RdpError {
    fn from(e: gfx::GraphicsPipelineError) -> Self {
        RdpError::GraphicsPipelineError(e)
    }
}

impl From<display::DisplayPipelineError> for RdpError {
    fn from(e: display::DisplayPipelineError) -> Self {
        RdpError::DisplayPipelineError(e)
    }
}

impl From<gfx::zgfx::ZgfxError> for RdpError {
    fn from(e: gfx::zgfx::ZgfxError) -> Self {
        RdpError::ZgfxError(e)
    }
}

impl From<FastPathError> for RdpError {
    fn from(e: FastPathError) -> Self {
        RdpError::FastPathError(e)
    }
}

impl From<ironrdp::RdpError> for RdpError {
    fn from(e: ironrdp::RdpError) -> Self {
        RdpError::RdpError(e)
    }
}

impl From<codecs::rfx::RfxError> for RdpError {
    fn from(e: codecs::rfx::RfxError) -> Self {
        RdpError::RfxError(e)
    }
}

impl From<codecs::rfx::rlgr::RlgrError> for RdpError {
    fn from(e: codecs::rfx::rlgr::RlgrError) -> Self {
        RdpError::RlgrError(e)
    }
}

impl From<ServerLicenseError> for RdpError {
    fn from(e: ServerLicenseError) -> Self {
        RdpError::ServerLicenseError(rdp::RdpError::ServerLicenseError(e))
    }
}
