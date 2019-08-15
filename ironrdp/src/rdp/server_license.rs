#[cfg(test)]
pub mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{impl_from_error, PduParsing};

const PREAMBLE_SIZE: usize = 4;
const ERROR_CODE_SIZE: usize = 4;
const STATE_TRANSITION_SIZE: usize = 4;
const BLOB_TYPE_SIZE: usize = 2;
const BLOB_LENGTH_SIZE: usize = 2;

const PROTOCOL_VERSION_MASK: u8 = 0x0F;

#[derive(Debug, Clone, PartialEq)]
pub struct ServerLicense {
    pub preamble: LicensePreamble,
    pub error_message: LicensingErrorMessage,
}

impl PduParsing for ServerLicense {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let preamble = LicensePreamble::from_buffer(&mut stream)?;
        let error_message = LicensingErrorMessage::from_buffer(&mut stream)?;

        Ok(Self {
            preamble,
            error_message,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.preamble
            .to_buffer(&mut stream, self.buffer_length() as u16)?;
        self.error_message.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.preamble.buffer_length() + self.error_message.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LicensePreamble {
    pub message_type: PreambleType,
    pub flags: PreambleFlags,
    pub version: PreambleVersion,
}

impl LicensePreamble {
    fn from_buffer(mut stream: impl io::Read) -> Result<Self, ServerLicenseError> {
        let message_type = PreambleType::from_u8(stream.read_u8()?)
            .ok_or(ServerLicenseError::InvalidLicenseType)?;
        let flags_with_version = stream.read_u8()?;
        let _packet_size = stream.read_u16::<LittleEndian>()?;

        let flags = PreambleFlags::from_bits(flags_with_version & !PROTOCOL_VERSION_MASK)
            .ok_or_else(|| {
                ServerLicenseError::InvalidPreamble(String::from("Got invalid flags field"))
            })?;
        let version = PreambleVersion::from_u8((flags_with_version & PROTOCOL_VERSION_MASK) as u8)
            .ok_or_else(|| {
                ServerLicenseError::InvalidPreamble(String::from(
                    "Got invalid version in the flags field",
                ))
            })?;

        match message_type {
            PreambleType::ErrorAlert => Ok(Self {
                message_type,
                flags,
                version,
            }),
            _ => Err(ServerLicenseError::InvalidPreamble(String::from(
                "Message type must be set to ERROR_ALERT",
            ))),
        }
    }

    fn to_buffer(
        &self,
        mut stream: impl io::Write,
        message_size: u16,
    ) -> Result<(), ServerLicenseError> {
        let flags_with_version = self.flags.bits() | self.version.to_u8().unwrap();

        stream.write_u8(self.message_type.to_u8().unwrap())?;
        stream.write_u8(flags_with_version)?;
        stream.write_u16::<LittleEndian>(message_size)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        PREAMBLE_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LicensingErrorMessage {
    pub error_code: LicensingErrorCode,
    pub state_transition: LicensingStateTransition,
    pub error_info: LicensingBinaryBlob,
}

impl PduParsing for LicensingErrorMessage {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let error_code = LicensingErrorCode::from_u32(stream.read_u32::<LittleEndian>()?)
            .ok_or(ServerLicenseError::InvalidErrorCode)?;
        let state_transition =
            LicensingStateTransition::from_u32(stream.read_u32::<LittleEndian>()?)
                .ok_or(ServerLicenseError::InvalidStateTransition)?;
        let error_info = LicensingBinaryBlob::from_buffer(&mut stream)?;

        Ok(Self {
            error_code,
            state_transition,
            error_info,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.error_code.to_u32().unwrap())?;
        stream.write_u32::<LittleEndian>(self.state_transition.to_u32().unwrap())?;
        self.error_info.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        ERROR_CODE_SIZE + STATE_TRANSITION_SIZE + self.error_info.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LicensingBinaryBlob {
    pub blob_type: BlobType,
    pub data: Vec<u8>,
}

impl PduParsing for LicensingBinaryBlob {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let blob_type = BlobType::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(ServerLicenseError::InvalidBlobType)?;
        let blob_len = stream.read_u16::<LittleEndian>()? as usize;

        let mut data = vec![0; blob_len];
        stream.read_exact(&mut data)?;

        Ok(Self { blob_type, data })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.blob_type.to_u16().unwrap())?;
        stream.write_u16::<LittleEndian>(self.data.len() as u16)?;
        stream.write_all(self.data.as_ref())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BLOB_TYPE_SIZE + BLOB_LENGTH_SIZE + self.data.len()
    }
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum PreambleType {
    LicenseRequest = 0x01,
    PlatformChallenge = 0x02,
    NewLicense = 0x03,
    UpgradeLicense = 0x04,
    LicenseInfo = 0x12,
    NewLicenseRequest = 0x13,
    PlatformChallengeResponse = 0x15,
    ErrorAlert = 0xff,
}

bitflags! {
    pub struct PreambleFlags: u8 {
        const EXTENDED_ERROR_MSG_SUPPORTED = 0x80;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum PreambleVersion {
    V2 = 2, // RDP 4.0
    V3 = 3, // RDP 5.0, 5.1, 5.2, 6.0, 6.1, 7.0, 7.1, 8.0, 8.1, 10.0, 10.1, 10.2, 10.3, 10.4, and 10.5
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum LicensingErrorCode {
    InvalidServerCertificate = 0x01,
    NoLicense = 0x02,
    InvalidMac = 0x03,
    InvalidScope = 0x4,
    NoLicenseServer = 0x06,
    StatusValidClient = 0x07,
    InvalidClient = 0x08,
    InvalidProductId = 0x0b,
    InvalidMessageLen = 0x0c,
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum LicensingStateTransition {
    TotalAbort = 1,
    NoTransition = 2,
    ResetPhaseToStart = 3,
    ResendLastMessage = 4,
}

#[derive(Debug, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum BlobType {
    Data = 0x01,
    Random = 0x02,
    Certificate = 0x03,
    Error = 0x04,
    EncryptedData = 0x09,
    KeyExchangeAlgorithm = 0x0d,
    Scope = 0x0e,
    ClientUserName = 0x0f,
    ClientMachineNameBlob = 0x10,
}

#[derive(Debug, Fail)]
pub enum ServerLicenseError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Invalid preamble field: {}", _0)]
    InvalidPreamble(String),
    #[fail(display = "Invalid preamble message type field")]
    InvalidLicenseType,
    #[fail(display = "Invalid error code field")]
    InvalidErrorCode,
    #[fail(display = "Invalid state transition field")]
    InvalidStateTransition,
    #[fail(display = "Invalid blob type field")]
    InvalidBlobType,
}

impl_from_error!(io::Error, ServerLicenseError, ServerLicenseError::IOError);
