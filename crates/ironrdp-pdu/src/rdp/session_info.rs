use std::io;

use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

use crate::{PduError, PduParsing};

#[cfg(test)]
mod tests;

mod logon_extended;
mod logon_info;

pub use self::logon_extended::{
    LogonErrorNotificationData, LogonErrorNotificationDataErrorCode, LogonErrorNotificationType, LogonErrorsInfo,
    LogonExFlags, LogonInfoExtended, ServerAutoReconnect,
};
pub use self::logon_info::{LogonInfo, LogonInfoVersion1, LogonInfoVersion2};

const INFO_TYPE_FIELD_SIZE: usize = 4;
const PLAIN_NOTIFY_PADDING_SIZE: usize = 576;
const PLAIN_NOTIFY_PADDING_BUFFER: [u8; PLAIN_NOTIFY_PADDING_SIZE] = [0; PLAIN_NOTIFY_PADDING_SIZE];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveSessionInfoPdu {
    pub info_type: InfoType,
    pub info_data: InfoData,
}

impl PduParsing for SaveSessionInfoPdu {
    type Error = SessionError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let info_type =
            InfoType::from_u32(stream.read_u32::<LittleEndian>()?).ok_or(SessionError::InvalidSaveSessionInfoType)?;

        let info_data = match info_type {
            InfoType::Logon => InfoData::LogonInfoV1(LogonInfoVersion1::from_buffer(&mut stream)?),
            InfoType::LogonLong => InfoData::LogonInfoV2(LogonInfoVersion2::from_buffer(&mut stream)?),
            InfoType::PlainNotify => {
                let mut padding_buffer = [0; PLAIN_NOTIFY_PADDING_SIZE];
                stream.read_exact(&mut padding_buffer)?;

                InfoData::PlainNotify
            }
            InfoType::LogonExtended => InfoData::LogonExtended(LogonInfoExtended::from_buffer(&mut stream)?),
        };

        Ok(Self { info_type, info_data })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.info_type.to_u32().unwrap())?;
        match self.info_data {
            InfoData::LogonInfoV1(ref info_v1) => {
                info_v1.to_buffer(&mut stream)?;
            }
            InfoData::LogonInfoV2(ref info_v2) => {
                info_v2.to_buffer(&mut stream)?;
            }
            InfoData::PlainNotify => {
                stream.write_all(PLAIN_NOTIFY_PADDING_BUFFER.as_ref())?;
            }
            InfoData::LogonExtended(ref extended) => {
                extended.to_buffer(&mut stream)?;
            }
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let info_data_size = match self.info_data {
            InfoData::LogonInfoV1(ref info_v1) => info_v1.buffer_length(),
            InfoData::LogonInfoV2(ref info_v2) => info_v2.buffer_length(),
            InfoData::PlainNotify => PLAIN_NOTIFY_PADDING_SIZE,
            InfoData::LogonExtended(ref extended) => extended.buffer_length(),
        };

        INFO_TYPE_FIELD_SIZE + info_data_size
    }
}

#[repr(u32)]
#[derive(Debug, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum InfoType {
    Logon = 0x0000_0000,
    LogonLong = 0x0000_0001,
    PlainNotify = 0x0000_0002,
    LogonExtended = 0x0000_0003,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoData {
    LogonInfoV1(LogonInfoVersion1),
    LogonInfoV2(LogonInfoVersion2),
    PlainNotify,
    LogonExtended(LogonInfoExtended),
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("invalid save session info type value")]
    InvalidSaveSessionInfoType,
    #[error("invalid domain name size value")]
    InvalidDomainNameSize,
    #[error("invalid user name size value")]
    InvalidUserNameSize,
    #[error("invalid logon version value")]
    InvalidLogonVersion2,
    #[error("invalid logon info version2 size value")]
    InvalidLogonVersion2Size,
    #[error("invalid server auto-reconnect packet size value")]
    InvalidAutoReconnectPacketSize,
    #[error("invalid server auto-reconnect version")]
    InvalidAutoReconnectVersion,
    #[error("invalid logon error type value")]
    InvalidLogonErrorType,
    #[error("invalid logon error data value")]
    InvalidLogonErrorData,
    #[error("PDU error: {0}")]
    Pdu(PduError),
}

impl From<PduError> for SessionError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}
