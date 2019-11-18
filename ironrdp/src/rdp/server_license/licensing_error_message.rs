#[cfg(test)]
mod test;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::{BlobHeader, BlobType, ServerLicenseError, BLOB_LENGTH_SIZE, BLOB_TYPE_SIZE};

use crate::PduParsing;

const ERROR_CODE_SIZE: usize = 4;
const STATE_TRANSITION_SIZE: usize = 4;

#[derive(Debug, PartialEq)]
pub struct LicensingErrorMessage {
    pub error_code: LicenseErrorCode,
    pub state_transition: LicensingStateTransition,
    pub error_info: Vec<u8>,
}

impl PduParsing for LicensingErrorMessage {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let error_code = LicenseErrorCode::from_u32(stream.read_u32::<LittleEndian>()?)
            .ok_or(ServerLicenseError::InvalidErrorCode)?;
        let state_transition =
            LicensingStateTransition::from_u32(stream.read_u32::<LittleEndian>()?)
                .ok_or(ServerLicenseError::InvalidStateTransition)?;

        let error_info_blob = BlobHeader::read_from_buffer(BlobType::Error, &mut stream)?;
        let error_info = vec![0u8; error_info_blob.length];

        Ok(Self {
            error_code,
            state_transition,
            error_info,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.error_code.to_u32().unwrap())?;
        stream.write_u32::<LittleEndian>(self.state_transition.to_u32().unwrap())?;

        BlobHeader::new(BlobType::Error, self.error_info.len()).write_to_buffer(&mut stream)?;
        stream.write_all(&self.error_info)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        ERROR_CODE_SIZE
            + STATE_TRANSITION_SIZE
            + self.error_info.len()
            + BLOB_LENGTH_SIZE
            + BLOB_TYPE_SIZE
    }
}

#[derive(Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum LicenseErrorCode {
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

#[derive(Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum LicensingStateTransition {
    TotalAbort = 1,
    NoTransition = 2,
    ResetPhaseToStart = 3,
    ResendLastMessage = 4,
}
