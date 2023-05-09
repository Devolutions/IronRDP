use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::SessionError;
use crate::PduParsing;

const LOGON_EX_LENGTH_FIELD_SIZE: usize = 2;
const LOGON_EX_FLAGS_FIELD_SIZE: usize = 4;
const LOGON_EX_PADDING_SIZE: usize = 570;
const LOGON_EX_PADDING_BUFFER: [u8; LOGON_EX_PADDING_SIZE] = [0; LOGON_EX_PADDING_SIZE];

const LOGON_INFO_FIELD_DATA_SIZE: usize = 4;
const AUTO_RECONNECT_VERSION_1: u32 = 0x0000_0001;
const AUTO_RECONNECT_PACKET_SIZE: usize = 28;
const AUTO_RECONNECT_RANDOM_BITS_SIZE: usize = 16;
const LOGON_ERRORS_INFO_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogonInfoExtended {
    pub present_fields_flags: LogonExFlags,
    pub auto_reconnect: Option<ServerAutoReconnect>,
    pub errors_info: Option<LogonErrorsInfo>,
}

impl LogonInfoExtended {
    fn get_internal_size(&self) -> usize {
        let reconnect_size = self.auto_reconnect.as_ref().map(|r| r.buffer_length()).unwrap_or(0);

        let errors_size = self.errors_info.as_ref().map(|r| r.buffer_length()).unwrap_or(0);

        LOGON_EX_LENGTH_FIELD_SIZE + LOGON_EX_FLAGS_FIELD_SIZE + reconnect_size + errors_size
    }
}

impl PduParsing for LogonInfoExtended {
    type Error = SessionError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _self_length = stream.read_u16::<LittleEndian>()?;
        let present_fields_flags = LogonExFlags::from_bits_truncate(stream.read_u32::<LittleEndian>()?);

        let auto_reconnect = if present_fields_flags.contains(LogonExFlags::AUTO_RECONNECT_COOKIE) {
            Some(ServerAutoReconnect::from_buffer(&mut stream)?)
        } else {
            None
        };

        let errors_info = if present_fields_flags.contains(LogonExFlags::LOGON_ERRORS) {
            Some(LogonErrorsInfo::from_buffer(&mut stream)?)
        } else {
            None
        };

        let mut padding_buffer = [0; LOGON_EX_PADDING_SIZE];
        stream.read_exact(&mut padding_buffer)?;

        Ok(Self {
            present_fields_flags,
            auto_reconnect,
            errors_info,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(self.get_internal_size() as u16)?;
        stream.write_u32::<LittleEndian>(self.present_fields_flags.bits())?;

        if let Some(ref reconnect) = self.auto_reconnect {
            reconnect.to_buffer(&mut stream)?;
        }
        if let Some(ref errors) = self.errors_info {
            errors.to_buffer(&mut stream)?;
        }

        stream.write_all(LOGON_EX_PADDING_BUFFER.as_ref())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.get_internal_size() + LOGON_EX_PADDING_SIZE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAutoReconnect {
    pub logon_id: u32,
    pub random_bits: [u8; AUTO_RECONNECT_RANDOM_BITS_SIZE],
}

impl PduParsing for ServerAutoReconnect {
    type Error = SessionError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _data_length = stream.read_u32::<LittleEndian>()?;
        let packet_length = stream.read_u32::<LittleEndian>()?;
        if packet_length != AUTO_RECONNECT_PACKET_SIZE as u32 {
            return Err(SessionError::InvalidAutoReconnectPacketSize);
        }

        let version = stream.read_u32::<LittleEndian>()?;
        if version != AUTO_RECONNECT_VERSION_1 {
            return Err(SessionError::InvalidAutoReconnectVersion);
        }

        let logon_id = stream.read_u32::<LittleEndian>()?;
        let mut random_bits = [0; AUTO_RECONNECT_RANDOM_BITS_SIZE];
        stream.read_exact(&mut random_bits)?;

        Ok(Self { logon_id, random_bits })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(AUTO_RECONNECT_PACKET_SIZE as u32)?;
        stream.write_u32::<LittleEndian>(AUTO_RECONNECT_PACKET_SIZE as u32)?;
        stream.write_u32::<LittleEndian>(AUTO_RECONNECT_VERSION_1)?;
        stream.write_u32::<LittleEndian>(self.logon_id)?;
        stream.write_all(self.random_bits.as_ref())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        AUTO_RECONNECT_PACKET_SIZE + LOGON_INFO_FIELD_DATA_SIZE
    }
}

/// TS_LOGON_ERRORS_INFO
///
/// [Doc](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/845eb789-6edf-453a-8b0e-c976823d1f72)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogonErrorsInfo {
    pub error_type: LogonErrorNotificationType,
    pub error_data: LogonErrorNotificationData,
}

impl PduParsing for LogonErrorsInfo {
    type Error = SessionError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _data_length = stream.read_u32::<LittleEndian>()?;
        let error_type = LogonErrorNotificationType::from_u32(stream.read_u32::<LittleEndian>()?)
            .ok_or(SessionError::InvalidLogonErrorType)?;

        let error_notification_data = stream.read_u32::<LittleEndian>()?;
        let error_data = LogonErrorNotificationDataErrorCode::from_u32(error_notification_data)
            .map(LogonErrorNotificationData::ErrorCode)
            .unwrap_or(LogonErrorNotificationData::SessionId(error_notification_data));

        Ok(Self { error_type, error_data })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(LOGON_ERRORS_INFO_SIZE as u32)?;
        stream.write_u32::<LittleEndian>(self.error_type.to_u32().unwrap())?;
        stream.write_u32::<LittleEndian>(self.error_data.to_u32())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        LOGON_ERRORS_INFO_SIZE + LOGON_INFO_FIELD_DATA_SIZE
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct LogonExFlags: u32 {
        const AUTO_RECONNECT_COOKIE = 0x0000_0001;
        const LOGON_ERRORS = 0x0000_0002;
    }
}

#[repr(u32)]
#[derive(Debug, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum LogonErrorNotificationType {
    SessionBusyOptions = 0xFFFF_FFF8,
    DisconnectRefused = 0xFFFF_FFF9,
    NoPermission = 0xFFFF_FFFA,
    BumpOptions = 0xFFFF_FFFB,
    ReconnectOptions = 0xFFFF_FFFC,
    SessionTerminate = 0xFFFF_FFFD,
    SessionContinue = 0xFFFF_FFFE,
    AccessDenied = 0xFFFF_FFFF,
}

#[repr(u32)]
#[derive(Debug, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum LogonErrorNotificationDataErrorCode {
    FailedBadPassword = 0x0000_0000,
    FailedUpdatePassword = 0x0000_0001,
    FailedOther = 0x0000_0002,
    Warning = 0x0000_0003,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogonErrorNotificationData {
    ErrorCode(LogonErrorNotificationDataErrorCode),
    SessionId(u32),
}

impl LogonErrorNotificationData {
    pub fn to_u32(&self) -> u32 {
        match self {
            LogonErrorNotificationData::ErrorCode(code) => code.to_u32().unwrap(),
            LogonErrorNotificationData::SessionId(id) => *id,
        }
    }
}
