use core::fmt;
use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

use crate::utils::CharacterSet;
use crate::{try_read_optional, try_write_optional, utils, PduParsing};

const RECONNECT_COOKIE_LEN: usize = 28;
const TIMEZONE_INFO_NAME_LEN: usize = 64;
const COMPRESSION_TYPE_MASK: u32 = 0x0000_1E00;

const CODE_PAGE_SIZE: usize = 4;
const FLAGS_SIZE: usize = 4;
const DOMAIN_LENGTH_SIZE: usize = 2;
const USER_NAME_LENGTH_SIZE: usize = 2;
const PASSWORD_LENGTH_SIZE: usize = 2;
const ALTERNATE_SHELL_LENGTH_SIZE: usize = 2;
const WORK_DIR_LENGTH_SIZE: usize = 2;

const CLIENT_ADDRESS_FAMILY_SIZE: usize = 2;
const CLIENT_ADDRESS_LENGTH_SIZE: usize = 2;
const CLIENT_DIR_LENGTH_SIZE: usize = 2;
const SESSION_ID_SIZE: usize = 4;
const PERFORMANCE_FLAGS_SIZE: usize = 4;
const RECONNECT_COOKIE_LENGTH_SIZE: usize = 2;
const BIAS_SIZE: usize = 4;
const SYSTEM_TIME_SIZE: usize = 16;

/// [2.2.1.11.1.1] Info Packet (TS_INFO_PACKET)
///
/// [2.2.1.11.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/732394f5-e2b5-4ac5-8a0a-35345386b0d1
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub credentials: Credentials,
    pub code_page: u32,
    pub flags: ClientInfoFlags,
    pub compression_type: CompressionType,
    pub alternate_shell: String,
    pub work_dir: String,
    pub extra_info: ExtendedClientInfo,
}

impl PduParsing for ClientInfo {
    type Error = ClientInfoError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let code_page = stream.read_u32::<LittleEndian>()?;
        let flags_with_compression_type = stream.read_u32::<LittleEndian>()?;

        let flags = ClientInfoFlags::from_bits(flags_with_compression_type & !COMPRESSION_TYPE_MASK)
            .ok_or(ClientInfoError::InvalidClientInfoFlags)?;
        let compression_type =
            CompressionType::from_u8(((flags_with_compression_type & COMPRESSION_TYPE_MASK) >> 9) as u8)
                .ok_or(ClientInfoError::InvalidClientInfoFlags)?;
        let character_set = if flags.contains(ClientInfoFlags::UNICODE) {
            CharacterSet::Unicode
        } else {
            CharacterSet::Ansi
        };

        // Sizes exclude the length of the mandatory null terminator
        let domain_size = stream.read_u16::<LittleEndian>()? as usize;
        let user_name_size = stream.read_u16::<LittleEndian>()? as usize;
        let password_size = stream.read_u16::<LittleEndian>()? as usize;
        let alternate_shell_size = stream.read_u16::<LittleEndian>()? as usize;
        let work_dir_size = stream.read_u16::<LittleEndian>()? as usize;

        let domain = utils::read_string_from_stream(&mut stream, domain_size, character_set, true)?;
        let username = utils::read_string_from_stream(&mut stream, user_name_size, character_set, true)?;
        let password = utils::read_string_from_stream(&mut stream, password_size, character_set, true)?;

        let domain = if domain.is_empty() { None } else { Some(domain) };
        let credentials = Credentials {
            username,
            password,
            domain,
        };

        let alternate_shell = utils::read_string_from_stream(&mut stream, alternate_shell_size, character_set, true)?;
        let work_dir = utils::read_string_from_stream(&mut stream, work_dir_size, character_set, true)?;

        let extra_info = ExtendedClientInfo::from_buffer(&mut stream, character_set)?;

        Ok(Self {
            credentials,
            code_page,
            flags,
            compression_type,
            alternate_shell,
            work_dir,
            extra_info,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let character_set = if self.flags.contains(ClientInfoFlags::UNICODE) {
            CharacterSet::Unicode
        } else {
            CharacterSet::Ansi
        };

        stream.write_u32::<LittleEndian>(self.code_page)?;

        let flags_with_compression_type = self.flags.bits() | (self.compression_type.to_u32().unwrap() << 9);
        stream.write_u32::<LittleEndian>(flags_with_compression_type)?;

        let domain = self.credentials.domain.clone().unwrap_or_default();
        stream.write_u16::<LittleEndian>(string_len(domain.as_str(), character_set))?;
        stream.write_u16::<LittleEndian>(string_len(self.credentials.username.as_str(), character_set))?;
        stream.write_u16::<LittleEndian>(string_len(self.credentials.password.as_str(), character_set))?;
        stream.write_u16::<LittleEndian>(string_len(self.alternate_shell.as_str(), character_set))?;
        stream.write_u16::<LittleEndian>(string_len(self.work_dir.as_str(), character_set))?;

        utils::write_string_with_null_terminator(&mut stream, domain.as_str(), character_set)?;
        utils::write_string_with_null_terminator(&mut stream, self.credentials.username.as_str(), character_set)?;
        utils::write_string_with_null_terminator(&mut stream, self.credentials.password.as_str(), character_set)?;
        utils::write_string_with_null_terminator(&mut stream, self.alternate_shell.as_str(), character_set)?;
        utils::write_string_with_null_terminator(&mut stream, self.work_dir.as_str(), character_set)?;

        self.extra_info.to_buffer(&mut stream, character_set)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let character_set = if self.flags.contains(ClientInfoFlags::UNICODE) {
            CharacterSet::Unicode
        } else {
            CharacterSet::Ansi
        };
        let domain = self.credentials.domain.clone().unwrap_or_default();

        CODE_PAGE_SIZE
            + FLAGS_SIZE
            + DOMAIN_LENGTH_SIZE
            + USER_NAME_LENGTH_SIZE
            + PASSWORD_LENGTH_SIZE
            + ALTERNATE_SHELL_LENGTH_SIZE
            + WORK_DIR_LENGTH_SIZE
            + (string_len(domain.as_str(), character_set)
                + string_len(self.credentials.username.as_str(), character_set)
                + string_len(self.credentials.password.as_str(), character_set)
                + string_len(self.alternate_shell.as_str(), character_set)
                + string_len(self.work_dir.as_str(), character_set)) as usize
            + character_set.to_usize().unwrap() * 5 // null terminator
            + self.extra_info.buffer_length(character_set)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub domain: Option<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // NOTE: do not show secret (user password)
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("domain", &self.domain)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedClientInfo {
    pub address_family: AddressFamily,
    pub address: String,
    pub dir: String,
    pub optional_data: ExtendedClientOptionalInfo,
}

impl ExtendedClientInfo {
    fn from_buffer(mut stream: impl io::Read, character_set: CharacterSet) -> Result<Self, ClientInfoError> {
        let address_family =
            AddressFamily::from_u16(stream.read_u16::<LittleEndian>()?).ok_or(ClientInfoError::InvalidAddressFamily)?;

        // This size includes the length of the mandatory null terminator.
        let address_size = stream.read_u16::<LittleEndian>()? as usize;
        let address = utils::read_string_from_stream(&mut stream, address_size, character_set, false)?;

        // This size includes the length of the mandatory null terminator.
        let dir_size = stream.read_u16::<LittleEndian>()? as usize;
        let dir = utils::read_string_from_stream(&mut stream, dir_size, character_set, false)?;

        let optional_data = ExtendedClientOptionalInfo::from_buffer(&mut stream)?;

        Ok(Self {
            address_family,
            address,
            dir,
            optional_data,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write, character_set: CharacterSet) -> Result<(), ClientInfoError> {
        stream.write_u16::<LittleEndian>(self.address_family.to_u16().unwrap())?;

        // + size of null terminator, which will write in the write_string function
        stream.write_u16::<LittleEndian>(
            string_len(self.address.as_str(), character_set) + character_set.to_u16().unwrap(),
        )?;
        utils::write_string_with_null_terminator(&mut stream, self.address.as_str(), character_set)?;

        stream.write_u16::<LittleEndian>(
            string_len(self.dir.as_str(), character_set) + character_set.to_u16().unwrap(),
        )?;
        utils::write_string_with_null_terminator(&mut stream, self.dir.as_str(), character_set)?;

        self.optional_data.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self, character_set: CharacterSet) -> usize {
        CLIENT_ADDRESS_FAMILY_SIZE
            + CLIENT_ADDRESS_LENGTH_SIZE
            + string_len(self.address.as_str(), character_set) as usize
            + character_set.to_usize().unwrap() // null terminator
        + CLIENT_DIR_LENGTH_SIZE
        + string_len(self.dir.as_str(), character_set) as usize
            + character_set.to_usize().unwrap() // null terminator
        + self.optional_data.buffer_length()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtendedClientOptionalInfo {
    timezone: Option<TimezoneInfo>,
    session_id: Option<u32>,
    performance_flags: Option<PerformanceFlags>,
    reconnect_cookie: Option<[u8; RECONNECT_COOKIE_LEN]>,
    // other fields are read by RdpVersion::Ten+
}

impl ExtendedClientOptionalInfo {
    /// Creates a new builder for [`ExtendedClientOptionalInfo`].
    pub fn builder(
    ) -> builder::ExtendedClientOptionalInfoBuilder<builder::ExtendedClientOptionalInfoBuilderStateSetTimeZone> {
        builder::ExtendedClientOptionalInfoBuilder::<builder::ExtendedClientOptionalInfoBuilderStateSetTimeZone> {
            inner: Self::default(),
            _phantom_data: Default::default(),
        }
    }

    pub fn timezone(&self) -> Option<&TimezoneInfo> {
        self.timezone.as_ref()
    }

    pub fn session_id(&self) -> Option<u32> {
        self.session_id
    }

    pub fn performance_flags(&self) -> Option<PerformanceFlags> {
        self.performance_flags
    }

    pub fn reconnect_cookie(&self) -> Option<&[u8; RECONNECT_COOKIE_LEN]> {
        self.reconnect_cookie.as_ref()
    }
}

impl PduParsing for ExtendedClientOptionalInfo {
    type Error = ClientInfoError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let mut optional_data = Self::default();

        optional_data.timezone = match TimezoneInfo::from_buffer(&mut stream) {
            Ok(v) => Some(v),
            Err(ClientInfoError::IOError(ref e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok(optional_data)
            }
            Err(e) => return Err(e),
        };
        optional_data.session_id = Some(try_read_optional!(stream.read_u32::<LittleEndian>(), optional_data));
        optional_data.performance_flags = Some(
            PerformanceFlags::from_bits(try_read_optional!(stream.read_u32::<LittleEndian>(), optional_data))
                .ok_or(ClientInfoError::InvalidPerformanceFlags)?,
        );

        let reconnect_cookie_size = try_read_optional!(stream.read_u16::<LittleEndian>(), optional_data);
        if reconnect_cookie_size != RECONNECT_COOKIE_LEN as u16 && reconnect_cookie_size != 0 {
            return Err(ClientInfoError::InvalidReconnectCookie);
        }
        if reconnect_cookie_size == 0 {
            return Ok(optional_data);
        }

        let mut reconnect_cookie = [0; RECONNECT_COOKIE_LEN];
        try_read_optional!(stream.read_exact(&mut reconnect_cookie), optional_data);
        optional_data.reconnect_cookie = Some(reconnect_cookie);

        try_read_optional!(stream.read_u16::<LittleEndian>(), optional_data); // reserved1
        try_read_optional!(stream.read_u16::<LittleEndian>(), optional_data); // reserved2

        Ok(optional_data)
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        try_write_optional!(self.timezone, |value: &TimezoneInfo| value.to_buffer(&mut stream));
        try_write_optional!(self.session_id, |value: &u32| stream.write_u32::<LittleEndian>(*value));
        try_write_optional!(self.performance_flags, |value: &PerformanceFlags| {
            stream.write_u32::<LittleEndian>(value.bits())
        });
        if let Some(reconnection_cookie) = self.reconnect_cookie {
            stream.write_u16::<LittleEndian>(reconnection_cookie.len() as u16)?;
            stream.write_all(reconnection_cookie.as_ref())?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let mut size = 0;

        if let Some(ref timezone) = self.timezone {
            size += timezone.buffer_length();
        }
        if self.session_id.is_some() {
            size += SESSION_ID_SIZE;
        }
        if self.performance_flags.is_some() {
            size += PERFORMANCE_FLAGS_SIZE;
        }
        if self.reconnect_cookie.is_some() {
            size += RECONNECT_COOKIE_LENGTH_SIZE + RECONNECT_COOKIE_LEN;
        }

        size
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimezoneInfo {
    pub bias: u32,
    pub standard_name: String,
    pub standard_date: Option<SystemTime>,
    pub standard_bias: u32,
    pub daylight_name: String,
    pub daylight_date: Option<SystemTime>,
    pub daylight_bias: u32,
}

impl PduParsing for TimezoneInfo {
    type Error = ClientInfoError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let bias = stream.read_u32::<LittleEndian>()?;

        let standard_name =
            utils::read_string_from_stream(&mut stream, TIMEZONE_INFO_NAME_LEN, CharacterSet::Unicode, false)?;
        let standard_date = Option::<SystemTime>::from_buffer(&mut stream)?;
        let standard_bias = stream.read_u32::<LittleEndian>()?;

        let daylight_name =
            utils::read_string_from_stream(&mut stream, TIMEZONE_INFO_NAME_LEN, CharacterSet::Unicode, false)?;
        let daylight_date = Option::<SystemTime>::from_buffer(&mut stream)?;
        let daylight_bias = stream.read_u32::<LittleEndian>()?;

        Ok(Self {
            bias,
            standard_name,
            standard_date,
            standard_bias,
            daylight_name,
            daylight_date,
            daylight_bias,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.bias)?;

        let mut standard_name = utils::to_utf16_bytes(self.standard_name.as_str());
        standard_name.resize(TIMEZONE_INFO_NAME_LEN, 0);
        stream.write_all(standard_name.as_ref())?;

        self.standard_date.to_buffer(&mut stream)?;
        stream.write_u32::<LittleEndian>(self.standard_bias)?;

        let mut daylight_name = utils::to_utf16_bytes(self.daylight_name.as_str());
        daylight_name.resize(TIMEZONE_INFO_NAME_LEN, 0);
        stream.write_all(daylight_name.as_ref())?;

        self.daylight_date.to_buffer(&mut stream)?;
        stream.write_u32::<LittleEndian>(self.daylight_bias)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BIAS_SIZE
            + TIMEZONE_INFO_NAME_LEN
            + self.standard_date.buffer_length()
            + BIAS_SIZE
            + TIMEZONE_INFO_NAME_LEN
            + self.daylight_date.buffer_length()
            + BIAS_SIZE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemTime {
    pub month: Month,
    pub day_of_week: DayOfWeek,
    pub day: DayOfWeekOccurrence,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

impl PduParsing for Option<SystemTime> {
    type Error = ClientInfoError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _year = stream.read_u16::<LittleEndian>()?; // This field MUST be set to zero.
        let month = stream.read_u16::<LittleEndian>()?;
        let day_of_week = stream.read_u16::<LittleEndian>()?;
        let day = stream.read_u16::<LittleEndian>()?;
        let hour = stream.read_u16::<LittleEndian>()?;
        let minute = stream.read_u16::<LittleEndian>()?;
        let second = stream.read_u16::<LittleEndian>()?;
        let milliseconds = stream.read_u16::<LittleEndian>()?;

        match (
            Month::from_u16(month),
            DayOfWeek::from_u16(day_of_week),
            DayOfWeekOccurrence::from_u16(day),
        ) {
            (Some(month), Some(day_of_week), Some(day)) => Ok(Some(SystemTime {
                month,
                day_of_week,
                day,
                hour,
                minute,
                second,
                milliseconds,
            })),
            _ => Ok(None),
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(0)?; // year
        match *self {
            Some(SystemTime {
                month,
                day_of_week,
                day,
                hour,
                minute,
                second,
                milliseconds,
            }) => {
                stream.write_u16::<LittleEndian>(month.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(day_of_week.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(day.to_u16().unwrap())?;
                stream.write_u16::<LittleEndian>(hour)?;
                stream.write_u16::<LittleEndian>(minute)?;
                stream.write_u16::<LittleEndian>(second)?;
                stream.write_u16::<LittleEndian>(milliseconds)?;
            }
            None => {
                stream.write_u16::<LittleEndian>(0)?;
                stream.write_u16::<LittleEndian>(0)?;
                stream.write_u16::<LittleEndian>(0)?;
                stream.write_u16::<LittleEndian>(0)?;
                stream.write_u16::<LittleEndian>(0)?;
                stream.write_u16::<LittleEndian>(0)?;
                stream.write_u16::<LittleEndian>(0)?;
            }
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        SYSTEM_TIME_SIZE
    }
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum Month {
    January = 1,
    February = 2,
    March = 3,
    April = 4,
    May = 5,
    June = 6,
    July = 7,
    August = 8,
    September = 9,
    October = 10,
    November = 11,
    December = 12,
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum DayOfWeek {
    Sunday = 0,
    Monday = 1,
    Tuesday = 2,
    Wednesday = 3,
    Thursday = 4,
    Friday = 5,
    Saturday = 6,
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum DayOfWeekOccurrence {
    First = 1,
    Second = 2,
    Third = 3,
    Fourth = 4,
    Last = 5,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PerformanceFlags: u32 {
        const DISABLE_WALLPAPER = 0x0000_0001;
        const DISABLE_FULLWINDOWDRAG = 0x0000_0002;
        const DISABLE_MENUANIMATIONS = 0x0000_0004;
        const DISABLE_THEMING = 0x0000_0008;
        const RESERVED1 = 0x0000_0010;
        const DISABLE_CURSOR_SHADOW = 0x0000_0020;
        const DISABLE_CURSORSETTINGS = 0x0000_0040;
        const ENABLE_FONT_SMOOTHING = 0x0000_0080;
        const ENABLE_DESKTOP_COMPOSITION = 0x0000_0100;
        const RESERVED2 = 0x8000_0000;
    }
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum AddressFamily {
    INet = 0x0002,
    INet6 = 0x0017,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClientInfoFlags: u32 {
        /// INFO_MOUSE
        const MOUSE = 0x0000_0001;
        /// INFO_DISABLECTRLALTDEL
        const DISABLE_CTRL_ALT_DEL = 0x0000_0002;
        /// INFO_AUTOLOGON
        const AUTOLOGON = 0x0000_0008;
        /// INFO_UNICODE
        const UNICODE = 0x0000_0010;
        /// INFO_MAXIMIZESHELL
        const MAXIMIZE_SHELL = 0x0000_0020;
        /// INFO_LOGONNOTIFY
        const LOGON_NOTIFY = 0x0000_0040;
        /// INFO_COMPRESSION
        const COMPRESSION = 0x0000_0080;
        /// INFO_ENABLEWINDOWSKEY
        const ENABLE_WINDOWS_KEY = 0x0000_0100;
        /// INFO_REMOTECONSOLEAUDIO
        const REMOTE_CONSOLE_AUDIO = 0x0000_2000;
        /// INFO_FORCE_ENCRYPTED_CS_PDU
        const FORCE_ENCRYPTED_CS_PDU = 0x0000_4000;
        /// INFO_RAIL
        const RAIL = 0x0000_8000;
        /// INFO_LOGONERRORS
        const LOGON_ERRORS = 0x0001_0000;
        /// INFO_MOUSE_HAS_WHEEL
        const MOUSE_HAS_WHEEL = 0x0002_0000;
        /// INFO_PASSWORD_IS_SC_PIN
        const PASSWORD_IS_SC_PIN = 0x0004_0000;
        /// INFO_NOAUDIOPLAYBACK
        const NO_AUDIO_PLAYBACK = 0x0008_0000;
        /// INFO_USING_SAVED_CREDS
        const USING_SAVED_CREDS = 0x0010_0000;
        /// INFO_AUDIOCAPTURE
        const AUDIO_CAPTURE = 0x0020_0000;
        /// INFO_VIDEO_DISABLE
        const VIDEO_DISABLE = 0x0040_0000;
        /// INFO_RESERVED1
        const RESERVED1 = 0x0080_0000;
        /// INFO_RESERVED1
        const RESERVED2 = 0x0100_0000;
        /// INFO_HIDEF_RAIL_SUPPORTED
        const HIDEF_RAIL_SUPPORTED = 0x0200_0000;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum CompressionType {
    K8 = 0,
    K64 = 1,
    Rdp6 = 2,
    Rdp61 = 3,
}

#[derive(Debug, Error)]
pub enum ClientInfoError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("UTF-8 error")]
    Utf8Error(#[from] std::string::FromUtf8Error),
    #[error("invalid address family field")]
    InvalidAddressFamily,
    #[error("invalid flags field")]
    InvalidClientInfoFlags,
    #[error("invalid performance flags field")]
    InvalidPerformanceFlags,
    #[error("invalid reconnect cookie field")]
    InvalidReconnectCookie,
}

fn string_len(value: &str, character_set: CharacterSet) -> u16 {
    match character_set {
        CharacterSet::Ansi => u16::try_from(value.len()).unwrap(),
        CharacterSet::Unicode => u16::try_from(value.encode_utf16().count() * 2).unwrap(),
    }
}

pub mod builder {
    use super::*;
    use std::marker::PhantomData;

    pub struct ExtendedClientOptionalInfoBuilderStateSetTimeZone;
    pub struct ExtendedClientOptionalInfoBuilderStateSetSessionId;
    pub struct ExtendedClientOptionalInfoBuilderStateSetPerformanceFlags;
    pub struct ExtendedClientOptionalInfoBuilderStateSetReconnectCookie;
    pub struct ExtendedClientOptionalInfoBuilderStateFinal;

    // State machine-based builder for [`ExtendedClientOptionalInfo`].
    //
    // [`ExtendedClientOptionalInfo`] strictly requires to set all preceding optional fields before
    // setting the next one, therefore we use a state machine to enforce this during the compile time.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ExtendedClientOptionalInfoBuilder<State> {
        pub(super) inner: ExtendedClientOptionalInfo,
        pub(super) _phantom_data: PhantomData<State>,
    }

    impl<State> ExtendedClientOptionalInfoBuilder<State> {
        pub fn build(self) -> ExtendedClientOptionalInfo {
            self.inner
        }
    }

    impl ExtendedClientOptionalInfoBuilder<ExtendedClientOptionalInfoBuilderStateSetTimeZone> {
        pub fn timezone(
            mut self,
            timezone: TimezoneInfo,
        ) -> ExtendedClientOptionalInfoBuilder<ExtendedClientOptionalInfoBuilderStateSetSessionId> {
            self.inner.timezone = Some(timezone);
            ExtendedClientOptionalInfoBuilder {
                inner: self.inner,
                _phantom_data: Default::default(),
            }
        }
    }

    impl ExtendedClientOptionalInfoBuilder<ExtendedClientOptionalInfoBuilderStateSetSessionId> {
        pub fn session_id(
            mut self,
            session_id: u32,
        ) -> ExtendedClientOptionalInfoBuilder<ExtendedClientOptionalInfoBuilderStateSetPerformanceFlags> {
            self.inner.session_id = Some(session_id);
            ExtendedClientOptionalInfoBuilder {
                inner: self.inner,
                _phantom_data: Default::default(),
            }
        }
    }

    impl ExtendedClientOptionalInfoBuilder<ExtendedClientOptionalInfoBuilderStateSetPerformanceFlags> {
        pub fn performance_flags(
            mut self,
            performance_flags: PerformanceFlags,
        ) -> ExtendedClientOptionalInfoBuilder<ExtendedClientOptionalInfoBuilderStateSetReconnectCookie> {
            self.inner.performance_flags = Some(performance_flags);
            ExtendedClientOptionalInfoBuilder {
                inner: self.inner,
                _phantom_data: Default::default(),
            }
        }
    }

    impl ExtendedClientOptionalInfoBuilder<ExtendedClientOptionalInfoBuilderStateSetReconnectCookie> {
        pub fn reconnect_cookie(
            mut self,
            reconnect_cookie: [u8; RECONNECT_COOKIE_LEN],
        ) -> ExtendedClientOptionalInfoBuilder<ExtendedClientOptionalInfoBuilderStateFinal> {
            self.inner.reconnect_cookie = Some(reconnect_cookie);
            ExtendedClientOptionalInfoBuilder {
                inner: self.inner,
                _phantom_data: Default::default(),
            }
        }
    }
}
