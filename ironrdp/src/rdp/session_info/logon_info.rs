use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::SessionError;
use crate::{utils, PduParsing};

const DOMAIN_NAME_SIZE_FIELD_SIZE: usize = 4;
const DOMAIN_NAME_SIZE_V1: usize = 52;
const USER_NAME_SIZE_FIELD_SIZE: usize = 4;
const USER_NAME_SIZE_V1: usize = 512;
const ID_SESSION_SIZE: usize = 4;

const SAVE_SESSION_PDU_VERSION_ONE: u16 = 0x0001;
const LOGON_INFO_V2_SIZE: usize = 18;
const LOGON_INFO_V2_PADDING_SIZE: usize = 558;
const LOGON_INFO_V2_PADDING_BUFFER: [u8; LOGON_INFO_V2_PADDING_SIZE] = [0; LOGON_INFO_V2_PADDING_SIZE];
const DOMAIN_NAME_SIZE_V2: usize = 52;
const USER_NAME_SIZE_V2: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct LogonInfoVersion1 {
    pub logon_info: LogonInfo,
}

impl PduParsing for LogonInfoVersion1 {
    type Error = SessionError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let domain_name_size = stream.read_u32::<LittleEndian>()?;
        if domain_name_size > DOMAIN_NAME_SIZE_V1 as u32 {
            return Err(SessionError::InvalidDomainNameSize);
        }

        let domain_name = utils::read_string(&mut stream, DOMAIN_NAME_SIZE_V1, utils::CharacterSet::Unicode, false)?;

        let user_name_size = stream.read_u32::<LittleEndian>()?;
        if user_name_size > USER_NAME_SIZE_V1 as u32 {
            return Err(SessionError::InvalidUserNameSize);
        }

        let user_name = utils::read_string(&mut stream, USER_NAME_SIZE_V1, utils::CharacterSet::Unicode, false)?;

        let session_id = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            logon_info: LogonInfo {
                session_id,
                domain_name,
                user_name,
            },
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let mut domain_name_buffer = utils::string_to_utf16(self.logon_info.domain_name.as_ref());
        domain_name_buffer.resize(DOMAIN_NAME_SIZE_V1 - 2, 0);
        let mut user_name_buffer = utils::string_to_utf16(self.logon_info.user_name.as_ref());
        user_name_buffer.resize(USER_NAME_SIZE_V1 - 2, 0);

        stream.write_u32::<LittleEndian>(((self.logon_info.domain_name.len() + 1) * 2) as u32)?;
        stream.write_all(domain_name_buffer.as_ref())?;
        stream.write_u16::<LittleEndian>(0)?; // UTF-16 null terminator
        stream.write_u32::<LittleEndian>(((self.logon_info.user_name.len() + 1) * 2) as u32)?;
        stream.write_all(user_name_buffer.as_ref())?;
        stream.write_u16::<LittleEndian>(0)?; // UTF-16 null terminator
        stream.write_u32::<LittleEndian>(self.logon_info.session_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        DOMAIN_NAME_SIZE_FIELD_SIZE
            + DOMAIN_NAME_SIZE_V1
            + USER_NAME_SIZE_FIELD_SIZE
            + USER_NAME_SIZE_V1
            + ID_SESSION_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogonInfoVersion2 {
    pub logon_info: LogonInfo,
}

impl PduParsing for LogonInfoVersion2 {
    type Error = SessionError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let version = stream.read_u16::<LittleEndian>()?;
        if version != SAVE_SESSION_PDU_VERSION_ONE {
            return Err(SessionError::InvalidLogonVersion2);
        }

        let size = stream.read_u32::<LittleEndian>()? as usize;
        if size != LOGON_INFO_V2_SIZE {
            return Err(SessionError::InvalidLogonVersion2Size);
        }

        let session_id = stream.read_u32::<LittleEndian>()?;
        let domain_name_size = stream.read_u32::<LittleEndian>()?;
        if domain_name_size > DOMAIN_NAME_SIZE_V2 as u32 {
            return Err(SessionError::InvalidDomainNameSize);
        }

        let user_name_size = stream.read_u32::<LittleEndian>()?;
        if user_name_size > USER_NAME_SIZE_V2 as u32 {
            return Err(SessionError::InvalidUserNameSize);
        }

        let mut padding_buffer = [0; LOGON_INFO_V2_PADDING_SIZE];
        stream.read_exact(&mut padding_buffer)?;

        let domain_name = utils::read_string(
            &mut stream,
            domain_name_size as usize,
            utils::CharacterSet::Unicode,
            false,
        )?;
        let user_name = utils::read_string(
            &mut stream,
            user_name_size as usize,
            utils::CharacterSet::Unicode,
            false,
        )?;

        Ok(Self {
            logon_info: LogonInfo {
                session_id,
                domain_name,
                user_name,
            },
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(SAVE_SESSION_PDU_VERSION_ONE)?;
        stream.write_u32::<LittleEndian>(LOGON_INFO_V2_SIZE as u32)?;
        stream.write_u32::<LittleEndian>(self.logon_info.session_id)?;
        stream.write_u32::<LittleEndian>(((self.logon_info.domain_name.len() + 1) * 2) as u32)?;
        stream.write_u32::<LittleEndian>(((self.logon_info.user_name.len() + 1) * 2) as u32)?;
        stream.write_all(LOGON_INFO_V2_PADDING_BUFFER.as_ref())?;

        utils::write_string_with_null_terminator(
            &mut stream,
            self.logon_info.domain_name.as_ref(),
            utils::CharacterSet::Unicode,
        )?;
        utils::write_string_with_null_terminator(
            &mut stream,
            self.logon_info.user_name.as_ref(),
            utils::CharacterSet::Unicode,
        )?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        LOGON_INFO_V2_SIZE
            + LOGON_INFO_V2_PADDING_SIZE
            + (self.logon_info.domain_name.len() + 1) * 2
            + (self.logon_info.user_name.len() + 1) * 2
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogonInfo {
    pub session_id: u32,
    pub user_name: String,
    pub domain_name: String,
}
