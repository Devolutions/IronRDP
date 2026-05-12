use core::fmt;
use std::io;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, invalid_field_err,
};

use crate::PduError;
use crate::input::InputEventError;
use crate::rdp::capability_sets::CapabilitySetsError;
use crate::rdp::client_info::{ClientInfo, ClientInfoError};
use crate::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags, ShareControlPduType, ShareDataPduType};
use crate::rdp::server_license::ServerLicenseError;

pub mod autodetect;
pub mod capability_sets;
pub mod client_info;
pub mod finalization_messages;
pub mod headers;
pub mod multitransport;
pub mod refresh_rectangle;
pub mod server_error_info;
pub mod server_license;
pub mod session_info;
pub mod suppress_output;
pub mod vc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfoPdu {
    pub security_header: BasicSecurityHeader,
    pub client_info: ClientInfo,
}

impl ClientInfoPdu {
    const NAME: &'static str = "ClientInfoPDU";

    const FIXED_PART_SIZE: usize = BasicSecurityHeader::FIXED_PART_SIZE + ClientInfo::FIXED_PART_SIZE;
}

impl Encode for ClientInfoPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.security_header.encode(dst)?;
        self.client_info.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.security_header.size() + self.client_info.size()
    }
}

impl<'de> Decode<'de> for ClientInfoPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let security_header = BasicSecurityHeader::decode(src)?;
        if !security_header.flags.contains(BasicSecurityHeaderFlags::INFO_PKT) {
            return Err(invalid_field_err!("securityHeader", "got invalid security header"));
        }

        let client_info = ClientInfo::decode(src)?;

        Ok(Self {
            security_header,
            client_info,
        })
    }
}

#[derive(Debug)]
pub enum RdpError {
    IOError(io::Error),
    ClientInfoError(ClientInfoError),
    ServerLicenseError(ServerLicenseError),
    CapabilitySetsError(CapabilitySetsError),
    InvalidSecurityHeader,
    InvalidShareControlHeader(String),
    InvalidShareDataHeader(String),
    InvalidPdu(String),
    UnexpectedShareControlPdu(ShareControlPduType),
    UnexpectedShareDataPdu(ShareDataPduType),
    SaveSessionInfoError(session_info::SessionError),
    InputEventError(InputEventError),
    NotEnoughBytes,
    Pdu(PduError),
}

impl fmt::Display for RdpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IOError(_) => f.write_str("IO error"),
            Self::ClientInfoError(_) => f.write_str("client Info PDU error"),
            Self::ServerLicenseError(_) => f.write_str("server License PDU error"),
            Self::CapabilitySetsError(_) => f.write_str("capability sets error"),
            Self::InvalidSecurityHeader => f.write_str("invalid RDP security header"),
            Self::InvalidShareControlHeader(s) => write!(f, "invalid RDP Share Control Header: {s}"),
            Self::InvalidShareDataHeader(s) => write!(f, "invalid RDP Share Data Header: {s}"),
            Self::InvalidPdu(_) => f.write_str("invalid RDP Connection Sequence PDU"),
            Self::UnexpectedShareControlPdu(ty) => write!(f, "unexpected RDP Share Control Header PDU type: {ty:?}"),
            Self::UnexpectedShareDataPdu(ty) => write!(f, "unexpected RDP Share Data Header PDU type: {ty:?}"),
            Self::SaveSessionInfoError(_) => f.write_str("save session info PDU error"),
            Self::InputEventError(_) => f.write_str("input event PDU error"),
            Self::NotEnoughBytes => f.write_str("not enough bytes"),
            Self::Pdu(e) => write!(f, "PDU error: {e}"),
        }
    }
}

impl core::error::Error for RdpError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::IOError(e) => Some(e),
            Self::ClientInfoError(e) => Some(e),
            Self::ServerLicenseError(e) => Some(e),
            Self::CapabilitySetsError(e) => Some(e),
            Self::SaveSessionInfoError(e) => Some(e),
            Self::InputEventError(e) => Some(e),
            Self::InvalidSecurityHeader
            | Self::InvalidShareControlHeader(_)
            | Self::InvalidShareDataHeader(_)
            | Self::InvalidPdu(_)
            | Self::UnexpectedShareControlPdu(_)
            | Self::UnexpectedShareDataPdu(_)
            | Self::NotEnoughBytes
            | Self::Pdu(_) => None,
        }
    }
}

impl From<io::Error> for RdpError {
    fn from(e: io::Error) -> Self {
        Self::IOError(e)
    }
}

impl From<ClientInfoError> for RdpError {
    fn from(e: ClientInfoError) -> Self {
        Self::ClientInfoError(e)
    }
}

impl From<ServerLicenseError> for RdpError {
    fn from(e: ServerLicenseError) -> Self {
        Self::ServerLicenseError(e)
    }
}

impl From<CapabilitySetsError> for RdpError {
    fn from(e: CapabilitySetsError) -> Self {
        Self::CapabilitySetsError(e)
    }
}

impl From<session_info::SessionError> for RdpError {
    fn from(e: session_info::SessionError) -> Self {
        Self::SaveSessionInfoError(e)
    }
}

impl From<InputEventError> for RdpError {
    fn from(e: InputEventError) -> Self {
        Self::InputEventError(e)
    }
}

impl From<PduError> for RdpError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}

impl From<RdpError> for io::Error {
    fn from(e: RdpError) -> io::Error {
        io::Error::other(format!("RDP Connection Sequence error: {e}"))
    }
}
