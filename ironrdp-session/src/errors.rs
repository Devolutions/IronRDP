use std::io;
use std::sync::mpsc::{RecvError, SendError};
use std::sync::PoisonError;

use ironrdp_core::dvc::{display, gfx};
use ironrdp_core::fast_path::FastPathError;
use ironrdp_core::rdp::server_license::ServerLicenseError;
use ironrdp_core::{codecs, nego, rdp, McsError};
use ironrdp_graphics::{rlgr, zgfx};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RdpError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("connection error")]
    ConnectionError(#[source] io::Error),
    #[error("X.224 error")]
    X224Error(#[source] io::Error),
    #[error("negotiation error")]
    NegotiationError(#[from] nego::NegotiationError),
    #[error("unexpected PDU: {0}")]
    UnexpectedPdu(String),
    #[error("Unexpected disconnection: {0}")]
    UnexpectedDisconnection(String),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    #[error("TLS connector error")]
    TlsConnectorError(#[source] native_tls::Error),
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    #[error("TLS handshake error")]
    TlsHandshakeError(#[source] native_tls::Error),
    #[cfg(feature = "rustls")]
    #[error("TLS connector error")]
    TlsConnectorError(#[source] rustls::Error),
    #[cfg(feature = "rustls")]
    #[error("TLS handshake error")]
    TlsHandshakeError(#[source] rustls::Error),
    #[error("CredSSP error")]
    CredSspError(#[from] sspi::Error),
    #[error("CredSSP TSRequest error")]
    TsRequestError(#[source] io::Error),
    #[error("early User Authentication Result error")]
    EarlyUserAuthResultError(#[source] io::Error),
    #[error("the server denied access via Early User Authentication Result")]
    AccessDenied,
    #[error("MCS Connect error")]
    McsConnectError(#[from] McsError),
    #[error("failed to get info about the user: {0}")]
    UserInfoError(String),
    #[error("MCS error")]
    McsError(#[source] McsError),
    #[error("Client Info PDU error")]
    ClientInfoError(#[source] rdp::RdpError),
    #[error("Server License PDU error")]
    ServerLicenseError(#[source] rdp::RdpError),
    #[error("Share Control Header error")]
    ShareControlHeaderError(#[source] rdp::RdpError),
    #[error("capability sets error")]
    CapabilitySetsError(#[source] rdp::RdpError),
    #[error("Virtual channel error")]
    VirtualChannelError(#[from] rdp::vc::ChannelError),
    #[error("Invalid channel id error: {0}")]
    InvalidChannelIdError(String),
    #[error("Graphics pipeline protocol error")]
    GraphicsPipelineError(#[from] gfx::GraphicsPipelineError),
    #[error("Display pipeline protocol error")]
    DisplayPipelineError(#[from] display::DisplayPipelineError),
    #[error("ZGFX error")]
    ZgfxError(#[from] zgfx::ZgfxError),
    #[error("Fast-Path error")]
    FastPathError(#[from] FastPathError),
    #[error("RDP error")]
    RdpError(#[from] ironrdp_core::RdpError),
    #[error("access to the non-existing channel: {0}")]
    AccessToNonExistingChannel(u32),
    #[error("access to the non-existing channel name: {0}")]
    AccessToNonExistingChannelName(String),
    #[error("data in unexpected channel: {0}")]
    UnexpectedChannel(u16),
    #[error("unexpected Surface Command codec ID: {0}")]
    UnexpectedCodecId(u8),
    #[error("RDP error")]
    RfxError(#[from] codecs::rfx::RfxError),
    #[error("absence of mandatory Fast-Path header")]
    MandatoryHeaderIsAbsent,
    #[error("RLGR error")]
    RlgrError(#[from] rlgr::RlgrError),
    #[error("absence of RFX channels")]
    NoRfxChannelsAnnounced,
    #[error("the server that started working using the inconsistent protocol: {0:?}")]
    UnexpectedFastPathUpdate(ironrdp_core::fast_path::UpdateCode),
    #[error("server error: {0}")]
    ServerError(String),
    #[error("Missing peer certificate")]
    MissingPeerCertificate,
    #[error("Dynamic virtual channel not connected")]
    DynamicVirtualChannelNotConnected,
    #[error("Static global channel not connected")]
    StaticChannelNotConnected,
    #[error("Invalid Capabilities mask provided. Mask: {0:X}")]
    InvalidCapabilitiesMask(u32),
    #[error("Stream terminated while waiting for some data")]
    UnexpectedStreamTermination,
    #[error("Unable to send message on channel {0}")]
    SendError(String),
    #[error("Unable to recieve message on channel {0}")]
    RecieveError(String),
    #[error("Lock poisoned")]
    LockPoisonedError,
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    #[error("Invalid DER structure")]
    DerEncode(#[source] native_tls::Error),
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

impl From<ServerLicenseError> for RdpError {
    fn from(e: ServerLicenseError) -> Self {
        RdpError::ServerLicenseError(rdp::RdpError::ServerLicenseError(e))
    }
}
