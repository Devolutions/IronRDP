use std::io;
use std::sync::mpsc::{RecvError, SendError};
use std::sync::PoisonError;

use ironrdp_core::dvc::{display, gfx};
use ironrdp_core::fast_path::FastPathError;
use ironrdp_core::rdp::server_license::ServerLicenseError;
use ironrdp_core::{codecs, rdp, McsError};
use ironrdp_graphics::{rlgr, zgfx};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RdpError {
    #[error("IO error")]
    Io(#[from] io::Error),
    #[error("connection error")]
    Connection(#[source] io::Error),
    #[error("X.224 error")]
    X224(#[source] io::Error),
    #[error("negotiation error")]
    Negotiation(#[from] ironrdp_core::NegotiationError),
    #[error("unexpected PDU: {0}")]
    UnexpectedPdu(String),
    #[error("Unexpected disconnection: {0}")]
    UnexpectedDisconnection(String),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    #[error("TLS connector error")]
    TlsConnector(#[source] async_native_tls::Error),
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    #[error("TLS handshake error")]
    TlsHandshake(#[source] async_native_tls::Error),
    #[cfg(feature = "rustls")]
    #[error("TLS connector error")]
    TlsConnector(#[source] tokio_rustls::rustls::Error),
    #[cfg(feature = "rustls")]
    #[error("TLS handshake error")]
    TlsHandshake(#[source] tokio_rustls::rustls::Error),
    #[error("CredSSP error")]
    CredSsp(#[from] sspi::Error),
    #[error("CredSSP TSRequest error")]
    TsRequest(#[source] io::Error),
    #[error("early User Authentication Result error")]
    EarlyUserAuthResult(#[source] io::Error),
    #[error("the server denied access via Early User Authentication Result")]
    AccessDenied,
    #[error("MCS Connect error")]
    McsConnect(#[from] McsError),
    #[error("failed to get info about the user: {0}")]
    UserInfo(String),
    #[error("MCS error")]
    Mcs(#[source] McsError),
    #[error("Client Info PDU error")]
    ClientInfo(#[source] rdp::RdpError),
    #[error("Server License PDU error")]
    ServerLicense(#[source] rdp::RdpError),
    #[error("Share Control Header error")]
    ShareControlHeader(#[source] rdp::RdpError),
    #[error("capability sets error")]
    CapabilitySets(#[source] rdp::RdpError),
    #[error("Virtual channel error")]
    VirtualChannel(#[from] rdp::vc::ChannelError),
    #[error("Invalid channel id error: {0}")]
    InvalidChannelId(String),
    #[error("Graphics pipeline protocol error")]
    GraphicsPipeline(#[from] gfx::GraphicsPipelineError),
    #[error("Display pipeline protocol error")]
    DisplayPipeline(#[from] display::DisplayPipelineError),
    #[error("ZGFX error")]
    Zgfx(#[from] zgfx::ZgfxError),
    #[error("Fast-Path error")]
    FastPath(#[from] FastPathError),
    #[error("RDP error")]
    Rdp(#[from] ironrdp_core::RdpError),
    #[error("access to the non-existing channel: {0}")]
    AccessToNonExistingChannel(u32),
    #[error("access to the non-existing channel name: {0}")]
    AccessToNonExistingChannelName(String),
    #[error("data in unexpected channel: {0}")]
    UnexpectedChannel(u16),
    #[error("unexpected Surface Command codec ID: {0}")]
    UnexpectedCodecId(u8),
    #[error("RDP error")]
    Rfx(#[from] codecs::rfx::RfxError),
    #[error("absence of mandatory Fast-Path header")]
    MandatoryHeaderIsAbsent,
    #[error("RLGR error")]
    Rlgr(#[from] rlgr::RlgrError),
    #[error("absence of RFX channels")]
    NoRfxChannelsAnnounced,
    #[error("the server that started working using the inconsistent protocol: {0:?}")]
    UnexpectedFastPathUpdate(ironrdp_core::fast_path::UpdateCode),
    #[error("server error: {0}")]
    Server(String),
    #[error("Missing peer certificate")]
    MissingPeerCertificate,
    #[error("Dynamic virtual channel not connected")]
    DynamicVirtualChannelNotConnected,
    #[error("Invalid Capabilities mask provided. Mask: {0:X}")]
    InvalidCapabilitiesMask(u32),
    #[error("Stream terminated while waiting for some data")]
    UnexpectedStreamTermination,
    #[error("Unable to send message on channel {0}")]
    Send(String),
    #[error("Unable to recieve message on channel {0}")]
    Recieve(String),
    #[error("Lock poisoned")]
    LockPoisoned,
    #[cfg(all(feature = "native-tls", not(feature = "rustls")))]
    #[error("Invalid DER structure")]
    DerEncode(#[source] async_native_tls::Error),
}

#[cfg(feature = "rustls")]
impl From<tokio_rustls::rustls::Error> for RdpError {
    fn from(e: tokio_rustls::rustls::Error) -> Self {
        match e {
            tokio_rustls::rustls::Error::InappropriateHandshakeMessage { .. } | tokio_rustls::rustls::Error::HandshakeNotComplete => {
                RdpError::TlsHandshake(e)
            }
            _ => RdpError::TlsConnector(e),
        }
    }
}

impl<T> From<SendError<T>> for RdpError {
    fn from(e: SendError<T>) -> Self {
        RdpError::Send(e.to_string())
    }
}

impl From<RecvError> for RdpError {
    fn from(e: RecvError) -> Self {
        RdpError::Recieve(e.to_string())
    }
}

impl<T> From<PoisonError<T>> for RdpError {
    fn from(_e: PoisonError<T>) -> Self {
        RdpError::LockPoisoned
    }
}

impl From<ServerLicenseError> for RdpError {
    fn from(e: ServerLicenseError) -> Self {
        RdpError::ServerLicense(rdp::RdpError::ServerLicenseError(e))
    }
}
