use std::io;

use thiserror::Error;

use self::client_info::ClientInfoError;
use self::finalization_messages::FinalizationMessagesError;
use self::server_license::ServerLicenseError;
use crate::input::InputEventError;
use crate::PduParsing;

#[cfg(test)]
pub mod test;

pub mod capability_sets;
pub mod server_license;
pub mod session_info;
pub mod vc;

mod client_info;
mod finalization_messages;
mod headers;
mod server_error_info;

pub use self::capability_sets::{
    CapabilitySet, CapabilitySetsError, ClientConfirmActive, DemandActive, ServerDemandActive, VirtualChannel,
    SERVER_CHANNEL_ID,
};
pub use self::client_info::{
    AddressFamily, ClientInfo, ClientInfoFlags, CompressionType, Credentials, DayOfWeek, DayOfWeekOccurrence,
    ExtendedClientInfo, ExtendedClientOptionalInfo, Month, PerformanceFlags, SystemTime, TimezoneInfo,
};
pub use self::finalization_messages::{
    ControlAction, ControlPdu, FontPdu, MonitorLayoutPdu, SequenceFlags, SynchronizePdu,
};
pub use self::headers::{
    BasicSecurityHeader, BasicSecurityHeaderFlags, CompressionFlags, ShareControlHeader, ShareControlPdu,
    ShareControlPduType, ShareDataHeader, ShareDataPdu, ShareDataPduType, StreamPriority, BASIC_SECURITY_HEADER_SIZE,
    SHARE_DATA_HEADER_COMPRESSION_MASK,
};
pub use self::server_error_info::{
    ErrorInfo, ProtocolIndependentCode, ProtocolIndependentConnectionBrokerCode, ProtocolIndependentLicensingCode,
    RdpSpecificCode, ServerSetErrorInfoError, ServerSetErrorInfoPdu,
};
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
    #[error("Client Info PDU error")]
    ClientInfoError(#[from] ClientInfoError),
    #[error("Server License PDU error")]
    ServerLicenseError(#[from] ServerLicenseError),
    #[error("Capability sets error")]
    CapabilitySetsError(#[from] CapabilitySetsError),
    #[error("Finalization PDUs error")]
    FinalizationMessagesError(#[from] FinalizationMessagesError),
    #[error("Invalid RDP security header")]
    InvalidSecurityHeader,
    #[error("Invalid RDP Share Control Header: {0}")]
    InvalidShareControlHeader(String),
    #[error("Invalid RDP Share Data Header: {0}")]
    InvalidShareDataHeader(String),
    #[error("Invalid RDP Connection Sequence PDU")]
    InvalidPdu(String),
    #[error("Unexpected RDP Share Control Header PDU type: {0:?}")]
    UnexpectedShareControlPdu(ShareControlPduType),
    #[error("Unexpected RDP Share Data Header PDU type: {0:?}")]
    UnexpectedShareDataPdu(ShareDataPduType),
    #[error("Save session info PDU error")]
    SaveSessionInfoError(#[from] session_info::SessionError),
    #[error("Server set error info PDU error")]
    ServerSetErrorInfoError(#[from] ServerSetErrorInfoError),
    #[error("Input event PDU error")]
    InputEventError(#[from] InputEventError),
    #[error("Not enough bytes")]
    NotEnoughBytes,
}

impl From<RdpError> for io::Error {
    fn from(e: RdpError) -> io::Error {
        io::Error::new(io::ErrorKind::Other, format!("RDP Connection Sequence error: {e}"))
    }
}
