use std::io;

use thiserror::Error;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::input::InputEventError;
use crate::rdp::capability_sets::CapabilitySetsError;
use crate::rdp::client_info::{ClientInfo, ClientInfoError};
use crate::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags, ShareControlPduType, ShareDataPduType};
use crate::rdp::server_license::ServerLicenseError;
use crate::{PduDecode, PduEncode, PduError, PduResult};

pub mod capability_sets;
pub mod client_info;
pub mod finalization_messages;
pub mod headers;
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

impl PduEncode for ClientInfoPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

impl<'de> PduDecode<'de> for ClientInfoPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let security_header = BasicSecurityHeader::decode(src)?;
        if !security_header.flags.contains(BasicSecurityHeaderFlags::INFO_PKT) {
            return Err(invalid_message_err!("securityHeader", "got invalid security header"));
        }

        let client_info = ClientInfo::decode(src)?;

        Ok(Self {
            security_header,
            client_info,
        })
    }
}

#[derive(Debug, Error)]
pub enum RdpError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("client Info PDU error")]
    ClientInfoError(#[from] ClientInfoError),
    #[error("server License PDU error")]
    ServerLicenseError(#[from] ServerLicenseError),
    #[error("capability sets error")]
    CapabilitySetsError(#[from] CapabilitySetsError),
    #[error("invalid RDP security header")]
    InvalidSecurityHeader,
    #[error("invalid RDP Share Control Header: {0}")]
    InvalidShareControlHeader(String),
    #[error("invalid RDP Share Data Header: {0}")]
    InvalidShareDataHeader(String),
    #[error("invalid RDP Connection Sequence PDU")]
    InvalidPdu(String),
    #[error("unexpected RDP Share Control Header PDU type: {0:?}")]
    UnexpectedShareControlPdu(ShareControlPduType),
    #[error("unexpected RDP Share Data Header PDU type: {0:?}")]
    UnexpectedShareDataPdu(ShareDataPduType),
    #[error("save session info PDU error")]
    SaveSessionInfoError(#[from] session_info::SessionError),
    #[error("input event PDU error")]
    InputEventError(#[from] InputEventError),
    #[error("not enough bytes")]
    NotEnoughBytes,
    #[error("PDU error: {0}")]
    Pdu(PduError),
}

impl From<PduError> for RdpError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}

impl From<RdpError> for io::Error {
    fn from(e: RdpError) -> io::Error {
        io::Error::new(io::ErrorKind::Other, format!("RDP Connection Sequence error: {e}"))
    }
}

#[cfg(feature = "std")]
impl ironrdp_error::legacy::ErrorContext for RdpError {
    fn context(&self) -> &'static str {
        "RDP"
    }
}
