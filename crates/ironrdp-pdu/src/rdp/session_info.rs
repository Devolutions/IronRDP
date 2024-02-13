use std::io;

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::{PduDecode, PduEncode, PduError, PduResult};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveSessionInfoPdu {
    pub info_type: InfoType,
    pub info_data: InfoData,
}

impl SaveSessionInfoPdu {
    const NAME: &'static str = "SaveSessionInfoPdu";

    const FIXED_PART_SIZE: usize = INFO_TYPE_FIELD_SIZE;
}

impl PduEncode for SaveSessionInfoPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.info_type.to_u32().unwrap());
        match self.info_data {
            InfoData::LogonInfoV1(ref info_v1) => {
                info_v1.encode(dst)?;
            }
            InfoData::LogonInfoV2(ref info_v2) => {
                info_v2.encode(dst)?;
            }
            InfoData::PlainNotify => {
                ensure_size!(in: dst, size: PLAIN_NOTIFY_PADDING_SIZE);
                write_padding!(dst, PLAIN_NOTIFY_PADDING_SIZE);
            }
            InfoData::LogonExtended(ref extended) => {
                extended.encode(dst)?;
            }
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let info_data_size = match self.info_data {
            InfoData::LogonInfoV1(ref info_v1) => info_v1.size(),
            InfoData::LogonInfoV2(ref info_v2) => info_v2.size(),
            InfoData::PlainNotify => PLAIN_NOTIFY_PADDING_SIZE,
            InfoData::LogonExtended(ref extended) => extended.size(),
        };

        Self::FIXED_PART_SIZE + info_data_size
    }
}

impl<'de> PduDecode<'de> for SaveSessionInfoPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let info_type = InfoType::from_u32(src.read_u32())
            .ok_or_else(|| invalid_message_err!("infoType", "invalid save session info type"))?;

        let info_data = match info_type {
            InfoType::Logon => InfoData::LogonInfoV1(LogonInfoVersion1::decode(src)?),
            InfoType::LogonLong => InfoData::LogonInfoV2(LogonInfoVersion2::decode(src)?),
            InfoType::PlainNotify => {
                ensure_size!(in: src, size: PLAIN_NOTIFY_PADDING_SIZE);
                read_padding!(src, PLAIN_NOTIFY_PADDING_SIZE);

                InfoData::PlainNotify
            }
            InfoType::LogonExtended => InfoData::LogonExtended(LogonInfoExtended::decode(src)?),
        };

        Ok(Self { info_type, info_data })
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
