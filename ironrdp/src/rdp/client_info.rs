#[cfg(test)]
pub mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::PduParsing;

const RECONNECT_COOKIE_LEN: usize = 28;
const TIMEZONE_INFO_NAME_LEN: usize = 64;
const COMPRESSION_TYPE_MASK: u32 = 0x0000_1E00;
const NULL_TERMINATOR: char = '\u{0}';

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

#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfo {
    pub credentials: sspi::Credentials,
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

        let flags =
            ClientInfoFlags::from_bits(flags_with_compression_type & !COMPRESSION_TYPE_MASK)
                .ok_or(ClientInfoError::InvalidClientInfoFlags)?;
        let compression_type = CompressionType::from_u8(
            ((flags_with_compression_type & COMPRESSION_TYPE_MASK) >> 9) as u8,
        )
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

        let domain = read_string(&mut stream, domain_size, character_set, true)?;
        let user_name = read_string(&mut stream, user_name_size, character_set, true)?;
        let password = read_string(&mut stream, password_size, character_set, true)?;

        let domain = if domain.is_empty() {
            None
        } else {
            Some(domain)
        };
        let credentials = sspi::Credentials::new(user_name, password, domain);

        let alternate_shell = read_string(&mut stream, alternate_shell_size, character_set, true)?;
        let work_dir = read_string(&mut stream, work_dir_size, character_set, true)?;

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

        let flags_with_compression_type =
            self.flags.bits() | (self.compression_type.to_u32().unwrap() << 9);
        stream.write_u32::<LittleEndian>(flags_with_compression_type)?;

        let domain = self.credentials.domain.clone().unwrap_or_default();
        stream.write_u16::<LittleEndian>(string_len(domain.as_str(), character_set))?;
        stream.write_u16::<LittleEndian>(string_len(
            self.credentials.username.as_str(),
            character_set,
        ))?;
        stream.write_u16::<LittleEndian>(string_len(
            self.credentials.password.as_str(),
            character_set,
        ))?;
        stream
            .write_u16::<LittleEndian>(string_len(self.alternate_shell.as_str(), character_set))?;
        stream.write_u16::<LittleEndian>(string_len(self.work_dir.as_str(), character_set))?;

        write_string_with_null_terminator(&mut stream, domain.as_str(), character_set)?;
        write_string_with_null_terminator(
            &mut stream,
            self.credentials.username.as_str(),
            character_set,
        )?;
        write_string_with_null_terminator(
            &mut stream,
            self.credentials.password.as_str(),
            character_set,
        )?;
        write_string_with_null_terminator(
            &mut stream,
            self.alternate_shell.as_str(),
            character_set,
        )?;
        write_string_with_null_terminator(&mut stream, self.work_dir.as_str(), character_set)?;

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

#[derive(Debug, Clone, PartialEq)]
pub struct ExtendedClientInfo {
    address_family: AddressFamily,
    address: String,
    dir: String,
    optional_data: ExtendedClientOptionalInfo,
}

impl ExtendedClientInfo {
    fn from_buffer(
        mut stream: impl io::Read,
        character_set: CharacterSet,
    ) -> Result<Self, ClientInfoError> {
        let address_family = AddressFamily::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(ClientInfoError::InvalidAddressFamily)?;

        // This size includes the length of the mandatory null terminator.
        let address_size = stream.read_u16::<LittleEndian>()? as usize;
        let address = read_string(&mut stream, address_size, character_set, false)?;

        // This size includes the length of the mandatory null terminator.
        let dir_size = stream.read_u16::<LittleEndian>()? as usize;
        let dir = read_string(&mut stream, dir_size, character_set, false)?;

        let optional_data = ExtendedClientOptionalInfo::from_buffer(&mut stream)?;

        Ok(Self {
            address_family,
            address,
            dir,
            optional_data,
        })
    }

    fn to_buffer(
        &self,
        mut stream: impl io::Write,
        character_set: CharacterSet,
    ) -> Result<(), ClientInfoError> {
        stream.write_u16::<LittleEndian>(self.address_family.to_u16().unwrap())?;

        // + size of null terminator, which will write in the write_string function
        stream.write_u16::<LittleEndian>(
            string_len(self.address.as_str(), character_set) + character_set.to_u16().unwrap(),
        )?;
        write_string_with_null_terminator(&mut stream, self.address.as_str(), character_set)?;

        stream.write_u16::<LittleEndian>(
            string_len(self.dir.as_str(), character_set) + character_set.to_u16().unwrap(),
        )?;
        write_string_with_null_terminator(&mut stream, self.dir.as_str(), character_set)?;

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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExtendedClientOptionalInfo {
    timezone: Option<TimezoneInfo>,
    session_id: Option<u32>,
    performance_flags: Option<PerformanceFlags>,
    reconnect_cookie: Option<[u8; RECONNECT_COOKIE_LEN]>,
    // other fields are read by RdpVersion::Ten+
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
        optional_data.session_id = Some(try_read_optional!(
            stream.read_u32::<LittleEndian>(),
            optional_data
        ));
        optional_data.performance_flags = Some(
            PerformanceFlags::from_bits(try_read_optional!(
                stream.read_u32::<LittleEndian>(),
                optional_data
            ))
            .ok_or(ClientInfoError::InvalidPerformanceFlags)?,
        );

        let reconnect_cookie_size =
            try_read_optional!(stream.read_u16::<LittleEndian>(), optional_data);
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
        try_write_optional!(self.timezone, |value: &TimezoneInfo| value
            .to_buffer(&mut stream));
        try_write_optional!(self.session_id, |value: &u32| stream
            .write_u32::<LittleEndian>(*value));
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

#[derive(Debug, Clone, PartialEq)]
pub struct TimezoneInfo {
    bias: u32,
    standard_name: String,
    standard_date: SystemTime,
    standard_bias: u32,
    daylight_name: String,
    daylight_date: SystemTime,
    daylight_bias: u32,
}

impl PduParsing for TimezoneInfo {
    type Error = ClientInfoError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let bias = stream.read_u32::<LittleEndian>()?;

        let standard_name = read_string(
            &mut stream,
            TIMEZONE_INFO_NAME_LEN,
            CharacterSet::Unicode,
            false,
        )?;
        let standard_date = SystemTime::from_buffer(&mut stream)?;
        let standard_bias = stream.read_u32::<LittleEndian>()?;

        let daylight_name = read_string(
            &mut stream,
            TIMEZONE_INFO_NAME_LEN,
            CharacterSet::Unicode,
            false,
        )?;
        let daylight_date = SystemTime::from_buffer(&mut stream)?;
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

        let mut standard_name = sspi::utils::string_to_utf16(self.standard_name.as_str());
        standard_name.resize(TIMEZONE_INFO_NAME_LEN, 0);
        stream.write_all(standard_name.as_ref())?;

        self.standard_date.to_buffer(&mut stream)?;
        stream.write_u32::<LittleEndian>(self.standard_bias)?;

        let mut daylight_name = sspi::utils::string_to_utf16(self.daylight_name.as_str());
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

#[derive(Debug, Clone, PartialEq)]
pub struct SystemTime {
    month: Month,
    day_of_week: DayOfWeek,
    day: DayOfWeekOccurrence,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

impl PduParsing for SystemTime {
    type Error = ClientInfoError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _year = stream.read_u16::<LittleEndian>()?; // This field MUST be set to zero.
        let month = Month::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(ClientInfoError::InvalidSystemTime)?;
        let day_of_week = DayOfWeek::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(ClientInfoError::InvalidSystemTime)?;
        let day = DayOfWeekOccurrence::from_u16(stream.read_u16::<LittleEndian>()?)
            .ok_or(ClientInfoError::InvalidSystemTime)?;
        let hour = stream.read_u16::<LittleEndian>()?;
        let minute = stream.read_u16::<LittleEndian>()?;
        let second = stream.read_u16::<LittleEndian>()?;
        let milliseconds = stream.read_u16::<LittleEndian>()?;

        Ok(Self {
            month,
            day_of_week,
            day,
            hour,
            minute,
            second,
            milliseconds,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u16::<LittleEndian>(0)?; // year
        stream.write_u16::<LittleEndian>(self.month.to_u16().unwrap())?;
        stream.write_u16::<LittleEndian>(self.day_of_week.to_u16().unwrap())?;
        stream.write_u16::<LittleEndian>(self.day.to_u16().unwrap())?;
        stream.write_u16::<LittleEndian>(self.hour)?;
        stream.write_u16::<LittleEndian>(self.minute)?;
        stream.write_u16::<LittleEndian>(self.second)?;
        stream.write_u16::<LittleEndian>(self.milliseconds)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        SYSTEM_TIME_SIZE
    }
}

#[repr(u16)]
#[derive(Debug, Clone, PartialEq, FromPrimitive, ToPrimitive)]
enum Month {
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
#[derive(Debug, Clone, PartialEq, FromPrimitive, ToPrimitive)]
enum DayOfWeek {
    Sunday = 0,
    Monday = 1,
    Tuesday = 2,
    Wednesday = 3,
    Thursday = 4,
    Friday = 5,
    Saturday = 6,
}

#[repr(u16)]
#[derive(Debug, Clone, PartialEq, FromPrimitive, ToPrimitive)]
enum DayOfWeekOccurrence {
    First = 1,
    Second = 2,
    Third = 3,
    Fourth = 4,
    Last = 5,
}

bitflags! {
    struct PerformanceFlags: u32 {
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
#[derive(Debug, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum AddressFamily {
    INet = 0x0002,
    INet6 = 0x0017,
}

bitflags! {
    pub struct ClientInfoFlags: u32 {
        const MOUSE = 0x0000_0001;
        const DISABLE_CTRL_ALT_DEL = 0x0000_0002;
        const AUTOLOGON = 0x0000_0008;
        const UNICODE = 0x0000_0010;
        const MAXIMIZE_SHELL = 0x0000_0020;
        const LOGON_NOTIFY = 0x0000_0040;
        const COMPRESSION = 0x0000_0080;
        const ENABLE_WINDOWS_KEY = 0x0000_0100;
        const REMOTE_CONSOLE_AUDIO = 0x0000_2000;
        const FORCE_ENCRYPTED_CS_PDU = 0x0000_4000;
        const RAIL = 0x0000_8000;
        const LOGON_ERRORS = 0x0001_0000;
        const MOUSE_HAS_WHEEL = 0x0002_0000;
        const PASSWORD_IS_SC_PIN = 0x0004_0000;
        const NO_AUDIO_PLAYBACK = 0x0008_0000;
        const USING_SAVED_CREDS = 0x0010_0000;
        const AUDIO_CAPTURE = 0x0020_0000;
        const VIDEO_DISABLE = 0x0040_0000;
        const RESERVED1 = 0x0080_0000;
        const RESERVED2 = 0x0100_0000;
        const HIDEF_RAIL_SUPPORTED = 0x0200_0000;
    }
}

#[derive(Debug, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum CompressionType {
    K8 = 0,
    K64 = 1,
    Rdp6 = 2,
    Rdp61 = 3,
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum CharacterSet {
    Ansi = 1,
    Unicode = 2,
}

#[derive(Debug, Fail)]
pub enum ClientInfoError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "UTF-8 error: {}", _0)]
    Utf8Error(#[fail(cause)] std::string::FromUtf8Error),
    #[fail(display = "Invalid address family field")]
    InvalidAddressFamily,
    #[fail(display = "Invalid flags field")]
    InvalidClientInfoFlags,
    #[fail(display = "Invalid performance flags field")]
    InvalidPerformanceFlags,
    #[fail(display = "Invalid reconnect cookie field")]
    InvalidReconnectCookie,
    #[fail(display = "Invalid system time field")]
    InvalidSystemTime,
}

impl_from_error!(io::Error, ClientInfoError, ClientInfoError::IOError);
impl_from_error!(
    std::string::FromUtf8Error,
    ClientInfoError,
    ClientInfoError::Utf8Error
);

fn string_len(value: &str, character_set: CharacterSet) -> u16 {
    value.len() as u16 * character_set.to_u16().unwrap()
}

fn read_string(
    mut stream: impl io::Read,
    size: usize,
    character_set: CharacterSet,
    read_null_terminator: bool,
) -> Result<String, ClientInfoError> {
    let size = size
        + if read_null_terminator {
            character_set.to_usize().unwrap()
        } else {
            0
        };
    let mut buffer = vec![0; size];
    stream.read_exact(&mut buffer)?;

    let result = match character_set {
        CharacterSet::Unicode => sspi::utils::bytes_to_utf16_string(buffer.as_slice()),
        CharacterSet::Ansi => String::from_utf8(buffer)?,
    };

    Ok(result.trim_end_matches(NULL_TERMINATOR).into())
}

fn write_string_with_null_terminator(
    mut stream: impl io::Write,
    value: &str,
    character_set: CharacterSet,
) -> io::Result<()> {
    match character_set {
        CharacterSet::Unicode => {
            stream.write_all(sspi::utils::string_to_utf16(value).as_ref())?;
            stream.write_u16::<LittleEndian>(0)
        }
        CharacterSet::Ansi => {
            stream.write_all(value.as_bytes())?;
            stream.write_u8(0)
        }
    }
}
