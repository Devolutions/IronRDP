#[cfg(test)]
pub mod test;

pub mod capability_sets;
pub mod server_license;
pub mod vc;

mod client_info;
mod finalization_messages;
mod headers;

pub use self::{
    capability_sets::{
        CapabilitySet, CapabilitySetsError, ClientConfirmActive, DemandActive, ServerDemandActive,
        VirtualChannel, SERVER_CHANNEL_ID,
    },
    client_info::{
        AddressFamily, ClientInfo, ClientInfoFlags, CompressionType, Credentials, DayOfWeek,
        DayOfWeekOccurrence, ExtendedClientInfo, ExtendedClientOptionalInfo, Month,
        PerformanceFlags, SystemTime, TimezoneInfo,
    },
    finalization_messages::{
        ControlAction, ControlPdu, FontPdu, MonitorLayoutPdu, SequenceFlags, SynchronizePdu,
    },
    headers::{
        BasicSecurityHeader, BasicSecurityHeaderFlags, CompressionFlags, ShareControlHeader,
        ShareControlPdu, ShareControlPduType, ShareDataHeader, ShareDataPdu, ShareDataPduType,
        StreamPriority, BASIC_SECURITY_HEADER_SIZE,
    },
};

use std::io;

use failure::Fail;

use self::{
    client_info::ClientInfoError, finalization_messages::FinalizationMessagesError,
    server_license::ServerLicenseError,
};
use crate::{impl_from_error, PduParsing};

#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfoPdu {
    pub security_header: BasicSecurityHeader,
    pub client_info: ClientInfo,
}

impl ClientInfoPdu {
    pub fn new(security_header: BasicSecurityHeader, client_info: ClientInfo) -> Self {
        Self {
            security_header,
            client_info,
        }
    }
}

impl PduParsing for ClientInfoPdu {
    type Error = RdpError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let security_header = BasicSecurityHeader::from_buffer(&mut stream)?;
        if security_header
            .flags
            .contains(BasicSecurityHeaderFlags::INFO_PKT)
        {
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

#[derive(Debug, Fail)]
pub enum RdpError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Client Info PDU error: {}", _0)]
    ClientInfoError(ClientInfoError),
    #[fail(display = "Server License PDU error: {}", _0)]
    ServerLicenseError(ServerLicenseError),
    #[fail(display = "Capability sets error: {}", _0)]
    CapabilitySetsError(CapabilitySetsError),
    #[fail(display = "Finalization PDUs error: {}", _0)]
    FinalizationMessagesError(FinalizationMessagesError),
    #[fail(display = "Invalid RDP security header")]
    InvalidSecurityHeader,
    #[fail(display = "Invalid RDP Share Control Header: {}", _0)]
    InvalidShareControlHeader(String),
    #[fail(display = "Invalid RDP Share Data Header: {}", _0)]
    InvalidShareDataHeader(String),
    #[fail(display = "Invalid RDP Connection Sequence PDU")]
    InvalidPdu(String),
    #[fail(display = "Unexpected RDP Share Control Header PDU type: {:?}", _0)]
    UnexpectedShareControlPdu(ShareControlPduType),
    #[fail(display = "Unexpected RDP Share Data Header PDU type: {:?}", _0)]
    UnexpectedShareDataPdu(ShareDataPduType),
}

impl_from_error!(io::Error, RdpError, RdpError::IOError);
impl_from_error!(ClientInfoError, RdpError, RdpError::ClientInfoError);
impl_from_error!(ServerLicenseError, RdpError, RdpError::ServerLicenseError);
impl_from_error!(CapabilitySetsError, RdpError, RdpError::CapabilitySetsError);
impl_from_error!(
    FinalizationMessagesError,
    RdpError,
    RdpError::FinalizationMessagesError
);

impl From<RdpError> for io::Error {
    fn from(e: RdpError) -> io::Error {
        io::Error::new(
            io::ErrorKind::Other,
            format!("RDP Connection Sequence error: {}", e),
        )
    }
}
