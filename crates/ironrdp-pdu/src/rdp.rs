use std::io;

use thiserror::Error;

use crate::input::InputEventError;
use crate::rdp::capability_sets::CapabilitySetsError;
use crate::rdp::client_info::{ClientInfo, ClientInfoError};
use crate::rdp::finalization_messages::FinalizationMessagesError;
use crate::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags, ShareControlPduType, ShareDataPduType};
use crate::rdp::server_error_info::ServerSetErrorInfoError;
use crate::rdp::server_license::ServerLicenseError;
use crate::PduParsing;

pub mod capability_sets;
pub mod client_info;
pub mod finalization_messages;
pub mod headers;
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

impl PduParsing for ClientInfoPdu {
    type Error = RdpError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let security_header = BasicSecurityHeader::from_buffer(&mut stream)?;
        if security_header.flags.contains(BasicSecurityHeaderFlags::INFO_PKT) {
            let client_info = ClientInfo::from_buffer(&mut stream)?;

            Ok(Self {
                security_header,
                client_info,
            })
        } else {
            Err(RdpError::InvalidPdu(String::from(
                "Expected ClientInfo PDU, got invalid SecurityHeader flags",
            )))
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.security_header.to_buffer(&mut stream)?;
        self.client_info.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.security_header.buffer_length() + self.client_info.buffer_length()
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
    #[error("finalization PDUs error")]
    FinalizationMessagesError(#[from] FinalizationMessagesError),
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
    #[error("server set error info PDU error")]
    ServerSetErrorInfoError(#[from] ServerSetErrorInfoError),
    #[error("input event PDU error")]
    InputEventError(#[from] InputEventError),
    #[error("not enough bytes")]
    NotEnoughBytes,
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
