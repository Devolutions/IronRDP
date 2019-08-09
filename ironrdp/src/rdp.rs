#[cfg(test)]
pub mod test;

mod capability_sets;
mod client_info;
mod client_license;
mod finalization_messages;
mod headers;

pub use self::{
    capability_sets::{
        CapabilitySet, CapabilitySetsError, ClientConfirmActive, DemandActive, ServerDemandActive,
        VirtualChannel,
    },
    finalization_messages::ControlAction,
    headers::{ShareControlHeader, ShareControlPdu, ShareDataHeader, ShareDataPdu},
};

use std::io;

use failure::Fail;

use self::{
    client_info::{ClientInfo, ClientInfoError},
    client_license::{ClientLicense, ClientLicenseError},
    finalization_messages::{
        ControlPdu, FinalizationMessagesError, MonitorLayoutPdu, SynchronizePdu,
    },
    headers::{
        BasicSecurityHeader, BasicSecurityHeaderFlags, ShareControlPduType, ShareDataPduType,
    },
};
use crate::PduParsing;

#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfoPdu {
    pub security_header: BasicSecurityHeader,
    pub client_info: ClientInfo,
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

#[derive(Debug, Clone, PartialEq)]
pub struct ClientLicensePdu {
    pub security_header: BasicSecurityHeader,
    pub client_license: ClientLicense,
}

impl PduParsing for ClientLicensePdu {
    type Error = RdpError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let security_header = BasicSecurityHeader::from_buffer(&mut stream)?;
        if security_header
            .flags
            .contains(BasicSecurityHeaderFlags::LICENSE_PKT)
        {
            let client_license = ClientLicense::from_buffer(&mut stream)?;

            Ok(Self {
                security_header,
                client_license,
            })
        } else {
            Err(RdpError::InvalidPdu(String::from(
                "Expected ClientLicense PDU, got invalid SecurityHeader flags",
            )))
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.security_header.to_buffer(&mut stream)?;
        self.client_license.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.security_header.buffer_length() + self.client_license.buffer_length()
    }
}

#[derive(Debug, Fail)]
pub enum RdpError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Client Info PDU error: {}", _0)]
    ClientInfoError(ClientInfoError),
    #[fail(display = "Client License PDU error: {}", _0)]
    ClientLicenseError(ClientLicenseError),
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
impl_from_error!(ClientLicenseError, RdpError, RdpError::ClientLicenseError);
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
