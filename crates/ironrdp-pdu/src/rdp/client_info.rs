use core::fmt;
use std::io;

use bitflags::bitflags;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

use crate::utils::CharacterSet;
use crate::{utils, PduDecode, PduEncode, PduError, PduResult};
use ironrdp_core::{ReadCursor, WriteCursor};

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

impl ClientInfo {
    const NAME: &'static str = "ClientInfo";

    pub const FIXED_PART_SIZE: usize = CODE_PAGE_SIZE
        + FLAGS_SIZE
        + DOMAIN_LENGTH_SIZE
        + USER_NAME_LENGTH_SIZE
        + PASSWORD_LENGTH_SIZE
        + ALTERNATE_SHELL_LENGTH_SIZE
        + WORK_DIR_LENGTH_SIZE;
}

impl PduEncode for ClientInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let character_set = if self.flags.contains(ClientInfoFlags::UNICODE) {
            CharacterSet::Unicode
        } else {
            CharacterSet::Ansi
        };

        dst.write_u32(self.code_page);

        let flags_with_compression_type = self.flags.bits() | (self.compression_type.to_u32().unwrap() << 9);
        dst.write_u32(flags_with_compression_type);

        let domain = self.credentials.domain.clone().unwrap_or_default();
        dst.write_u16(string_len(domain.as_str(), character_set));
        dst.write_u16(string_len(self.credentials.username.as_str(), character_set));
        dst.write_u16(string_len(self.credentials.password.as_str(), character_set));
        dst.write_u16(string_len(self.alternate_shell.as_str(), character_set));
        dst.write_u16(string_len(self.work_dir.as_str(), character_set));

        utils::write_string_to_cursor(dst, domain.as_str(), character_set, true)?;
        utils::write_string_to_cursor(dst, self.credentials.username.as_str(), character_set, true)?;
        utils::write_string_to_cursor(dst, self.credentials.password.as_str(), character_set, true)?;
        utils::write_string_to_cursor(dst, self.alternate_shell.as_str(), character_set, true)?;
        utils::write_string_to_cursor(dst, self.work_dir.as_str(), character_set, true)?;

        self.extra_info.encode(dst, character_set)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
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
            + self.extra_info.size(character_set)
    }
}

impl<'de> PduDecode<'de> for ClientInfo {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let code_page = src.read_u32();
        let flags_with_compression_type = src.read_u32();

        let flags = ClientInfoFlags::from_bits(flags_with_compression_type & !COMPRESSION_TYPE_MASK)
            .ok_or(invalid_message_err!("flags", "invalid ClientInfoFlags"))?;
        let compression_type =
            CompressionType::from_u8(((flags_with_compression_type & COMPRESSION_TYPE_MASK) >> 9) as u8)
                .ok_or(invalid_message_err!("flags", "invalid CompressionType"))?;

        let character_set = if flags.contains(ClientInfoFlags::UNICODE) {
            CharacterSet::Unicode
        } else {
            CharacterSet::Ansi
        };

        // Sizes exclude the length of the mandatory null terminator
        let nt = character_set.to_usize().unwrap();
        let domain_size = src.read_u16() as usize + nt;
        let user_name_size = src.read_u16() as usize + nt;
        let password_size = src.read_u16() as usize + nt;
        let alternate_shell_size = src.read_u16() as usize + nt;
        let work_dir_size = src.read_u16() as usize + nt;
        ensure_size!(in: src, size: domain_size + user_name_size + password_size + alternate_shell_size + work_dir_size);

        let domain = utils::decode_string(src.read_slice(domain_size), character_set, true)?;
        let username = utils::decode_string(src.read_slice(user_name_size), character_set, true)?;
        let password = utils::decode_string(src.read_slice(password_size), character_set, true)?;

        let domain = if domain.is_empty() { None } else { Some(domain) };
        let credentials = Credentials {
            username,
            password,
            domain,
        };

        let alternate_shell = utils::decode_string(src.read_slice(alternate_shell_size), character_set, true)?;
        let work_dir = utils::decode_string(src.read_slice(work_dir_size), character_set, true)?;

        let extra_info = ExtendedClientInfo::decode(src, character_set)?;

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
    // const NAME: &'static str = "ExtendedClientInfo";

    fn decode(src: &mut ReadCursor<'_>, character_set: CharacterSet) -> PduResult<Self> {
        ensure_size!(in: src, size: CLIENT_ADDRESS_FAMILY_SIZE + CLIENT_ADDRESS_LENGTH_SIZE);

        let address_family = AddressFamily::from_u16(src.read_u16())
            .ok_or(invalid_message_err!("clientAddressFamily", "invalid address family"))?;

        // This size includes the length of the mandatory null terminator.
        let address_size = src.read_u16() as usize;
        ensure_size!(in: src, size: address_size + CLIENT_DIR_LENGTH_SIZE);

        let address = utils::decode_string(src.read_slice(address_size), character_set, false)?;
        // This size includes the length of the mandatory null terminator.
        let dir_size = src.read_u16() as usize;
        ensure_size!(in: src, size: dir_size);

        let dir = utils::decode_string(src.read_slice(dir_size), character_set, false)?;

        let optional_data = ExtendedClientOptionalInfo::decode(src)?;

        Ok(Self {
            address_family,
            address,
            dir,
            optional_data,
        })
    }

    fn encode(&self, dst: &mut WriteCursor<'_>, character_set: CharacterSet) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size(character_set));

        dst.write_u16(self.address_family.to_u16().unwrap());
        // // + size of null terminator, which will write in the write_string function
        dst.write_u16(string_len(self.address.as_str(), character_set) + character_set.to_u16().unwrap());
        utils::write_string_to_cursor(dst, self.address.as_str(), character_set, true)?;
        dst.write_u16(string_len(self.dir.as_str(), character_set) + character_set.to_u16().unwrap());
        utils::write_string_to_cursor(dst, self.dir.as_str(), character_set, true)?;
        self.optional_data.encode(dst)?;

        Ok(())
    }

    fn size(&self, character_set: CharacterSet) -> usize {
        CLIENT_ADDRESS_FAMILY_SIZE
            + CLIENT_ADDRESS_LENGTH_SIZE
            + string_len(self.address.as_str(), character_set) as usize
            + character_set.to_usize().unwrap() // null terminator
        + CLIENT_DIR_LENGTH_SIZE
        + string_len(self.dir.as_str(), character_set) as usize
            + character_set.to_usize().unwrap() // null terminator
        + self.optional_data.size()
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
    const NAME: &'static str = "ExtendedClientOptionalInfo";

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

impl PduEncode for ExtendedClientOptionalInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        if let Some(ref timezone) = self.timezone {
            timezone.encode(dst)?;
        }
        if let Some(session_id) = self.session_id {
            dst.write_u32(session_id);
        }
        if let Some(performance_flags) = self.performance_flags {
            dst.write_u32(performance_flags.bits());
        }
        if let Some(reconnect_cookie) = self.reconnect_cookie {
            dst.write_u16(RECONNECT_COOKIE_LEN as u16);
            dst.write_array(reconnect_cookie);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let mut size = 0;

        if let Some(ref timezone) = self.timezone {
            size += timezone.size();
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

impl<'de> PduDecode<'de> for ExtendedClientOptionalInfo {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let mut optional_data = Self::default();

        if src.len() < TimezoneInfo::FIXED_PART_SIZE {
            return Ok(optional_data);
        }
        optional_data.timezone = Some(TimezoneInfo::decode(src)?);

        if src.len() < 4 {
            return Ok(optional_data);
        }
        optional_data.session_id = Some(src.read_u32());

        if src.len() < 4 {
            return Ok(optional_data);
        }
        optional_data.performance_flags = Some(
            PerformanceFlags::from_bits(src.read_u32())
                .ok_or(invalid_message_err!("performanceFlags", "invalid performance flags"))?,
        );

        if src.len() < 2 {
            return Ok(optional_data);
        }
        let reconnect_cookie_size = src.read_u16();
        if reconnect_cookie_size != RECONNECT_COOKIE_LEN as u16 && reconnect_cookie_size != 0 {
            return Err(invalid_message_err!("cbAutoReconnectCookie", "invalid cookie size"));
        }
        if reconnect_cookie_size != 0 {
            if src.len() < RECONNECT_COOKIE_LEN {
                return Err(invalid_message_err!("cbAutoReconnectCookie", "missing cookie data"));
            }
            optional_data.reconnect_cookie = Some(src.read_array());
        }

        if src.len() < 2 * 2 {
            return Ok(optional_data);
        }
        src.read_u16(); // reserved1
        src.read_u16(); // reserved2

        Ok(optional_data)
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

impl TimezoneInfo {
    const NAME: &'static str = "TimezoneInfo";

    const FIXED_PART_SIZE: usize = BIAS_SIZE
        + TIMEZONE_INFO_NAME_LEN
        + SystemTime::FIXED_PART_SIZE
        + BIAS_SIZE
        + TIMEZONE_INFO_NAME_LEN
        + SystemTime::FIXED_PART_SIZE
        + BIAS_SIZE;
}

impl PduEncode for TimezoneInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.bias);

        let mut standard_name = utils::to_utf16_bytes(self.standard_name.as_str());
        standard_name.resize(TIMEZONE_INFO_NAME_LEN, 0);
        dst.write_slice(&standard_name);

        self.standard_date.encode(dst)?;
        dst.write_u32(self.standard_bias);

        let mut daylight_name = utils::to_utf16_bytes(self.daylight_name.as_str());
        daylight_name.resize(TIMEZONE_INFO_NAME_LEN, 0);
        dst.write_slice(&daylight_name);

        self.daylight_date.encode(dst)?;
        dst.write_u32(self.daylight_bias);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for TimezoneInfo {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let bias = src.read_u32();
        let standard_name = utils::decode_string(src.read_slice(TIMEZONE_INFO_NAME_LEN), CharacterSet::Unicode, false)?;
        let standard_date = <Option<SystemTime>>::decode(src)?;
        let standard_bias = src.read_u32();

        let daylight_name = utils::decode_string(src.read_slice(TIMEZONE_INFO_NAME_LEN), CharacterSet::Unicode, false)?;
        let daylight_date = <Option<SystemTime>>::decode(src)?;
        let daylight_bias = src.read_u32();

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

impl SystemTime {
    const NAME: &'static str = "SystemTime";

    const FIXED_PART_SIZE: usize = 2 /* Year */ + 2 /* Month */ + 2 /* DoW */ + 2 /* Day */ + 2 /* Hour */ + 2 /* Minute */ + 2 /* Second */ + 2 /* Ms */;
}

impl PduEncode for Option<SystemTime> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(0); // year
        if let Some(st) = self {
            dst.write_u16(st.month.to_u16().unwrap());
            dst.write_u16(st.day_of_week.to_u16().unwrap());
            dst.write_u16(st.day.to_u16().unwrap());
            dst.write_u16(st.hour);
            dst.write_u16(st.minute);
            dst.write_u16(st.second);
            dst.write_u16(st.milliseconds);
        } else {
            write_padding!(dst, 2 * 7);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        SystemTime::NAME
    }

    fn size(&self) -> usize {
        SystemTime::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Option<SystemTime> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_size!(in: src, size: SystemTime::FIXED_PART_SIZE);

        let _year = src.read_u16(); // This field MUST be set to zero.
        let month = src.read_u16();
        let day_of_week = src.read_u16();
        let day = src.read_u16();
        let hour = src.read_u16();
        let minute = src.read_u16();
        let second = src.read_u16();
        let milliseconds = src.read_u16();

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

impl Default for PerformanceFlags {
    fn default() -> Self {
        Self::DISABLE_FULLWINDOWDRAG | Self::DISABLE_MENUANIMATIONS | Self::ENABLE_FONT_SMOOTHING
    }
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum AddressFamily {
    Invalid = 0x0000, // value given by MS Remote Desktop for Android 10.0.18.1251
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
    #[error("PDU error: {0}")]
    Pdu(PduError),
}

impl From<PduError> for ClientInfoError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
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
