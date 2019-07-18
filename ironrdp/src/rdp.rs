#[cfg(test)]
pub mod test;

mod client_info;
mod client_license;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;

use self::{
    client_info::{ClientInfo, ClientInfoError},
    client_license::{ClientLicense, ClientLicenseError},
};
use crate::PduParsing;

const BASIC_SECURITY_HEADER_SIZE: usize = 4;

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

#[derive(Debug, Clone, PartialEq)]
pub struct BasicSecurityHeader {
    flags: BasicSecurityHeaderFlags,
}

impl PduParsing for BasicSecurityHeader {
    type Error = RdpError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let flags = BasicSecurityHeaderFlags::from_bits(stream.read_u16::<LittleEndian>()?)
            .ok_or(RdpError::InvalidSecurityHeader)?;
        let _flags_hi = stream.read_u16::<LittleEndian>()?; // unused

        Ok(Self { flags })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.flags.bits())?;
        stream.write_u16::<LittleEndian>(0)?; // flags_hi

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BASIC_SECURITY_HEADER_SIZE
    }
}

bitflags! {
    pub struct BasicSecurityHeaderFlags: u16 {
        const EXCHANGE_PKT = 0x0001;
        const TRANSPORT_REQ = 0x0002;
        const TRANSPORT_RSP = 0x0004;
        const ENCRYPT = 0x0008;
        const RESET_SEQNO = 0x0010;
        const IGNORE_SEQNO = 0x0020;
        const INFO_PKT = 0x0040;
        const LICENSE_PKT = 0x0080;
        const LICENSE_ENCRYPT_CS = 0x0100;
        const LICENSE_ENCRYPT_SC = 0x0200;
        const REDIRECTION_PKT = 0x0400;
        const SECURE_CHECKSUM = 0x0800;
        const AUTODETECT_REQ = 0x1000;
        const AUTODETECT_RSP = 0x2000;
        const HEARTBEAT = 0x4000;
        const FLAGSHI_VALID = 0x8000;
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
    #[fail(display = "Invalid RDP security header")]
    InvalidSecurityHeader,
    #[fail(display = "Invalid RDP Connection Sequence PDU")]
    InvalidPdu(String),
}

impl_from_error!(io::Error, RdpError, RdpError::IOError);
impl_from_error!(ClientInfoError, RdpError, RdpError::ClientInfoError);
impl_from_error!(ClientLicenseError, RdpError, RdpError::ClientLicenseError);

impl From<RdpError> for io::Error {
    fn from(e: RdpError) -> io::Error {
        io::Error::new(
            io::ErrorKind::Other,
            format!("RDP Connection Sequence error: {}", e),
        )
    }
}
