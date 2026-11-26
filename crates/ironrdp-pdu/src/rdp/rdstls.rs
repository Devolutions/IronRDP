//! RDSTLS PDUs, exchanged over TLS in place of CredSSP when `PROTOCOL_RDSTLS` is negotiated.
//!
//! Defined in [\[MS-RDPBCGR\] 2.2.17 RDSTLS PDUs].
//!
//! [\[MS-RDPBCGR\] 2.2.17 RDSTLS PDUs]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/2aa353a2-2e46-4bb7-84ef-9354eb07a25f

use core::fmt;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err,
};

use crate::utils::{self, CharacterSet};

/// `RDSTLS_VERSION_1`, the only version this module encodes and accepts.
pub const RDSTLS_VERSION_1: u16 = 0x0001;

const TYPE_CAPABILITIES: u16 = 0x0001;
const TYPE_AUTHREQ: u16 = 0x0002;
const TYPE_AUTHRSP: u16 = 0x0004;

const DATA_CAPABILITIES: u16 = 0x0001;
const DATA_PASSWORD_CREDS: u16 = 0x0001;
const DATA_RESULT_CODE: u16 = 0x0001;

const HEADER_SIZE: usize = 2 /* Version */ + 2 /* PduType */ + 2 /* DataType */;

/// [2.2.17.1] RDSTLS Capabilities PDU
///
/// Sent by the server to advertise the RDSTLS versions it supports.
///
/// [2.2.17.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/35910151-abed-463b-aaf0-dd95010c2dc6
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdstlsCapabilities {
    /// Bit mask of the supported versions, such as [`RDSTLS_VERSION_1`].
    pub supported_versions: u16,
}

impl RdstlsCapabilities {
    const NAME: &'static str = "RdstlsCapabilities";

    const FIXED_PART_SIZE: usize = HEADER_SIZE + 2 /* SupportedVersions */;
}

impl Encode for RdstlsCapabilities {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        write_header(dst, TYPE_CAPABILITIES, DATA_CAPABILITIES);
        dst.write_u16(self.supported_versions);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for RdstlsCapabilities {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        read_header(src, TYPE_CAPABILITIES, DATA_CAPABILITIES)?;
        let supported_versions = src.read_u16();

        Ok(Self { supported_versions })
    }
}

/// [2.2.17.2] RDSTLS Authentication Request PDU with Password Credentials
///
/// Sent by the client to authenticate with the credentials of a Server Redirection Packet.
///
/// [2.2.17.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/d0c153e4-2727-4f9f-a149-765b7dc52c31
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdstlsPasswordAuthenticationRequest {
    /// `RedirectionGuid` of the Server Redirection Packet, as received.
    pub redirection_guid: Vec<u8>,
    /// User name; the encoder adds the null terminator, so an empty name is sent as a lone terminator.
    pub username: String,
    /// Domain; the encoder adds the null terminator, so an empty domain is sent as a lone terminator.
    pub domain: String,
    /// Encrypted password blob of the Server Redirection Packet, as received.
    pub password: Vec<u8>,
}

impl RdstlsPasswordAuthenticationRequest {
    const NAME: &'static str = "RdstlsPasswordAuthenticationRequest";

    const FIXED_PART_SIZE: usize = HEADER_SIZE
        + 2 /* RedirectionGuidLength */
        + 2 /* UserNameLength */
        + 2 /* DomainLength */
        + 2 /* PasswordLength */;
}

impl fmt::Debug for RdstlsPasswordAuthenticationRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The GUID and the password are redacted to their lengths: they authenticate the redirected connection.
        f.debug_struct("RdstlsPasswordAuthenticationRequest")
            .field("redirection_guid", &self.redirection_guid.len())
            .field("username", &self.username)
            .field("domain", &self.domain)
            .field("password", &self.password.len())
            .finish()
    }
}

impl Encode for RdstlsPasswordAuthenticationRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        write_header(dst, TYPE_AUTHREQ, DATA_PASSWORD_CREDS);
        write_bytes_field(dst, &self.redirection_guid)?;
        write_unicode_field(dst, &self.username)?;
        write_unicode_field(dst, &self.domain)?;
        write_bytes_field(dst, &self.password)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            + self.redirection_guid.len()
            + unicode_size(&self.username)
            + unicode_size(&self.domain)
            + self.password.len()
    }
}

impl<'de> Decode<'de> for RdstlsPasswordAuthenticationRequest {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: HEADER_SIZE);
        read_header(src, TYPE_AUTHREQ, DATA_PASSWORD_CREDS)?;

        let redirection_guid = read_bytes_field(src)?.to_vec();
        let username = read_unicode_field(src)?;
        let domain = read_unicode_field(src)?;
        let password = read_bytes_field(src)?.to_vec();

        Ok(Self {
            redirection_guid,
            username,
            domain,
            password,
        })
    }
}

/// [2.2.17.4] RDSTLS Authentication Response PDU
///
/// Sent by the server with the result of user authentication.
///
/// [2.2.17.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/96327ab4-d43f-4803-9aff-392ce1fc2073
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdstlsAuthenticationResponse {
    pub result_code: RdstlsResultCode,
}

impl RdstlsAuthenticationResponse {
    const NAME: &'static str = "RdstlsAuthenticationResponse";

    const FIXED_PART_SIZE: usize = HEADER_SIZE + 4 /* ResultCode */;
}

impl Encode for RdstlsAuthenticationResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        write_header(dst, TYPE_AUTHRSP, DATA_RESULT_CODE);
        dst.write_u32(self.result_code.0);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for RdstlsAuthenticationResponse {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        read_header(src, TYPE_AUTHRSP, DATA_RESULT_CODE)?;
        let result_code = RdstlsResultCode(src.read_u32());

        Ok(Self { result_code })
    }
}

/// `ResultCode` of an [`RdstlsAuthenticationResponse`].
///
/// Values outside the named constants are kept as received.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdstlsResultCode(pub u32);

impl RdstlsResultCode {
    /// User authentication succeeded.
    pub const SUCCESS: Self = Self(0x0000_0000);
    /// The user does not have permission to access the server.
    pub const ACCESS_DENIED: Self = Self(0x0000_0005);
    /// The username is unknown or the supplied password is incorrect.
    pub const LOGON_FAILURE: Self = Self(0x0000_052E);
    /// The user account has time restrictions and cannot be accessed at this time.
    pub const INVALID_LOGON_HOURS: Self = Self(0x0000_0530);
    /// The password associated with the user account has expired.
    pub const PASSWORD_EXPIRED: Self = Self(0x0000_0532);
    /// The user account is currently disabled.
    pub const ACCOUNT_DISABLED: Self = Self(0x0000_0533);
    /// The password associated with the user account has to be changed.
    pub const PASSWORD_MUST_CHANGE: Self = Self(0x0000_0773);
    /// The user account is currently locked out and cannot be accessed.
    pub const ACCOUNT_LOCKED_OUT: Self = Self(0x0000_0775);

    fn name(self) -> Option<&'static str> {
        let name = match self {
            Self::SUCCESS => "RDSTLS_RESULT_SUCCESS",
            Self::ACCESS_DENIED => "RDSTLS_RESULT_ACCESS_DENIED",
            Self::LOGON_FAILURE => "RDSTLS_RESULT_LOGON_FAILURE",
            Self::INVALID_LOGON_HOURS => "RDSTLS_RESULT_INVALID_LOGON_HOURS",
            Self::PASSWORD_EXPIRED => "RDSTLS_RESULT_PASSWORD_EXPIRED",
            Self::ACCOUNT_DISABLED => "RDSTLS_RESULT_ACCOUNT_DISABLED",
            Self::PASSWORD_MUST_CHANGE => "RDSTLS_RESULT_PASSWORD_MUST_CHANGE",
            Self::ACCOUNT_LOCKED_OUT => "RDSTLS_RESULT_ACCOUNT_LOCKED_OUT",
            _ => return None,
        };
        Some(name)
    }
}

impl fmt::Debug for RdstlsResultCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "RdstlsResultCode({:#010X})", self.0),
        }
    }
}

impl fmt::Display for RdstlsResultCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => write!(f, "{name} ({:#010X})", self.0),
            None => write!(f, "{:#010X}", self.0),
        }
    }
}

fn write_header(dst: &mut WriteCursor<'_>, pdu_type: u16, data_type: u16) {
    dst.write_u16(RDSTLS_VERSION_1);
    dst.write_u16(pdu_type);
    dst.write_u16(data_type);
}

fn read_header(src: &mut ReadCursor<'_>, pdu_type: u16, data_type: u16) -> DecodeResult<()> {
    if src.read_u16() != RDSTLS_VERSION_1 {
        return Err(invalid_field_err!("Version", "unsupported RDSTLS version", in: src));
    }
    if src.read_u16() != pdu_type {
        return Err(invalid_field_err!("PduType", "unexpected RDSTLS PDU type", in: src));
    }
    if src.read_u16() != data_type {
        return Err(invalid_field_err!("DataType", "unexpected RDSTLS data type", in: src));
    }
    Ok(())
}

fn unicode_size(value: &str) -> usize {
    utils::encoded_str_len(value, CharacterSet::Unicode, true)
}

fn write_unicode_field(dst: &mut WriteCursor<'_>, value: &str) -> EncodeResult<()> {
    dst.write_u16(cast_length!("length", unicode_size(value), in: dst)?);
    utils::write_string_to_cursor(dst, value, CharacterSet::Unicode, true)
}

fn write_bytes_field(dst: &mut WriteCursor<'_>, value: &[u8]) -> EncodeResult<()> {
    dst.write_u16(cast_length!("length", value.len(), in: dst)?);
    dst.write_slice(value);
    Ok(())
}

fn read_bytes_field<'de>(src: &mut ReadCursor<'de>) -> DecodeResult<&'de [u8]> {
    ensure_size!(in: src, size: 2 /* Length */);
    let length = usize::from(src.read_u16());
    ensure_size!(in: src, size: length);
    Ok(src.read_slice(length))
}

/// Reads a length-prefixed Unicode string, dropping its null terminator.
fn read_unicode_field(src: &mut ReadCursor<'_>) -> DecodeResult<String> {
    let bytes = read_bytes_field(src)?;
    if bytes.len() % 2 != 0 {
        return Err(invalid_field_err!("length", "odd length of a Unicode string", in: src));
    }
    let text = utils::decode_string(bytes, CharacterSet::Unicode, false)?;
    Ok(text.trim_end_matches('\0').to_owned())
}
