//! String types

use alloc::string::String;

use crate::{PduDecode, PduEncode, PduResult, ReadCursor, VarU32, WriteCursor};

/// String value up to 2^32 bytes long.
///
/// NOW-PROTO: NOW_LRGSTR
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowLrgStr(String);

impl NowLrgStr {
    pub const MAX_SIZE: usize = u32::MAX as usize;

    const NAME: &'static str = "NOW_LRGSTR";
    const FIXED_PART_SIZE: usize = 4;

    /// Returns empty string.
    pub fn empty() -> Self {
        Self(String::new())
    }

    /// Creates new `NowLrgStr`. Returns error if string is too big for the protocol.
    pub fn new(value: impl Into<String>) -> PduResult<Self> {
        let value = value.into();
        // IMPORTANT: we need to check for encoded UTF-8 size, not the string length.

        let _: u32 = value
            .as_bytes()
            .len()
            .try_into()
            .map_err(|_| invalid_message_err!("string value", "too large string"))?;

        Ok(NowLrgStr(value))
    }

    pub fn value(&self) -> &str {
        &self.0
    }
}

impl PduEncode for NowLrgStr {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let encoded_size = self.size();
        ensure_size!(in: dst, size: encoded_size);

        let len: u32 = self.0.len().try_into().expect("BUG: validated in constructor");

        dst.write_u32(len);
        dst.write_slice(self.0.as_bytes());
        dst.write_u8(b'\0');

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE  /* u32 size */
            + self.0.len() /* utf-8 bytes */
            + 1 /* null termnator */
    }
}

impl PduDecode<'_> for NowLrgStr {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let len: usize = cast_length!("len", src.read_u32())?;

        ensure_size!(in: src, size: len);
        let bytes = src.read_slice(len);
        ensure_size!(in: src, size: 1);
        let _null = src.read_u8();

        let string =
            String::from_utf8(bytes.to_vec()).map_err(|_| invalid_message_err!("string value", "invalid utf-8"))?;

        Ok(NowLrgStr(string))
    }
}

impl From<NowLrgStr> for String {
    fn from(value: NowLrgStr) -> Self {
        value.0
    }
}

/// String value up to 2^31 bytes long (Length has compact variable length encoding).
///
/// NOW-PROTO: NOW_VARSTR
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowVarStr(String);

impl NowVarStr {
    pub const MAX_SIZE: usize = VarU32::MAX as usize;

    const NAME: &'static str = "NOW_VARSTR";

    /// Returns empty string.
    pub fn empty() -> Self {
        Self(String::new())
    }

    /// Creates `NowVarStr` from std string. Returns error if string is too big for the protocol.
    pub fn new(value: impl Into<String>) -> PduResult<Self> {
        let value = value.into();
        // IMPORTANT: we need to check for encoded UTF-8 size, not the string length.

        let _: u32 = value
            .as_bytes()
            .len()
            .try_into()
            .ok()
            .and_then(|val| if val <= VarU32::MAX { Some(val) } else { None })
            .ok_or_else(|| invalid_message_err!("string value", "too large string"))?;

        Ok(NowVarStr(value))
    }

    pub fn value(&self) -> &str {
        &self.0
    }
}

impl PduEncode for NowVarStr {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let encoded_size = self.size();
        ensure_size!(in: dst, size: encoded_size);

        let len: u32 = self.0.len().try_into().expect("BUG: validated in constructor");

        VarU32::new(len)?.encode(dst)?;
        dst.write_slice(self.0.as_bytes());
        dst.write_u8(b'\0');

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        VarU32::new(self.0.len().try_into().unwrap()).unwrap().size() /* variable-length size */
            + self.0.len() /* utf-8 bytes */
            + 1 /* null termnator */
    }
}

impl PduDecode<'_> for NowVarStr {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let len_u32 = VarU32::decode(src)?.value();
        let len: usize = cast_length!("len", len_u32)?;

        ensure_size!(in: src, size: len);
        let bytes = src.read_slice(len);
        ensure_size!(in: src, size: 1);
        let _null = src.read_u8();

        let string =
            String::from_utf8(bytes.to_vec()).map_err(|_| invalid_message_err!("string value", "invalid utf-8"))?;

        Ok(NowVarStr(string))
    }
}

impl From<NowVarStr> for String {
    fn from(value: NowVarStr) -> Self {
        value.0
    }
}

const fn restricted_str_name(str_len: u8) -> &'static str {
    match str_len {
        15 => "NOW_STRING16",
        31 => "NOW_STRING32",
        63 => "NOW_STRING64",
        127 => "NOW_STRING128",
        255 => "NOW_STRING256",
        _ => panic!("BUG: Requested restricted string variant is not defined in the protocol"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowRestrictedStr<const MAX_LEN: u8>(String);

impl<const MAX_LEN: u8> NowRestrictedStr<MAX_LEN> {
    pub const MAX_ENCODED_UTF8_LEN: usize = MAX_LEN as usize;

    const NAME: &'static str = restricted_str_name(MAX_LEN);
    const FIXED_PART_SIZE: usize = 1;

    /// Returns empty string.
    pub fn empty() -> Self {
        Self(String::new())
    }

    /// Creates `NowRestrictedStr` from std string. Returns error if string is too big for the protocol.
    pub fn new(value: impl Into<String>) -> PduResult<Self> {
        let value = value.into();

        // IMPORTANT: we need to check for encoded UTF-8 size, not the string length
        if value.as_bytes().len() > MAX_LEN as usize {
            return Err(invalid_message_err!("string value", concat!("too large string")));
        }
        Ok(NowRestrictedStr(value))
    }

    pub fn value(&self) -> &str {
        &self.0
    }
}

impl<const MAX_LEN: u8> PduEncode for NowRestrictedStr<MAX_LEN> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let encoded_size = self.size();
        ensure_size!(in: dst, size: encoded_size);

        let len: u8 = self.0.len().try_into().expect("BUG: validated in constructor");

        dst.write_u8(len);
        dst.write_slice(self.0.as_bytes());
        dst.write_u8(b'\0');

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE  /* u8 size */
            + self.0.len() /* utf-8 bytes */
            + 1 /* null termnator */
    }
}

impl<const MAX_LEN: u8> PduDecode<'_> for NowRestrictedStr<MAX_LEN> {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let len = src.read_u8();
        if len > MAX_LEN {
            return Err(invalid_message_err!("string value", "too large string"));
        }

        let len_usize = len.into();

        ensure_size!(in: src, size: len_usize);
        let bytes = src.read_slice(len_usize);
        ensure_size!(in: src, size: 1);
        let _null = src.read_u8();

        let string =
            String::from_utf8(bytes.to_vec()).map_err(|_| invalid_message_err!("string value", "invalid utf-8"))?;

        Ok(NowRestrictedStr(string))
    }
}

impl<const N: u8> From<NowRestrictedStr<N>> for String {
    fn from(value: NowRestrictedStr<N>) -> Self {
        value.0
    }
}

/// String value up to 16 bytes long.
///
/// NOW-PROTO: NOW_STRING16
pub type NowString16 = NowRestrictedStr<15>;

/// String value up to 32 bytes long.
///
/// NOW-PROTO: NOW_STRING32
pub type NowString32 = NowRestrictedStr<31>;

/// String value up to 64 bytes long.
///
/// NOW-PROTO: NOW_STRING64
pub type NowString64 = NowRestrictedStr<63>;

/// String value up to 128 bytes long.
///
/// NOW-PROTO: NOW_STRING128
pub type NowString128 = NowRestrictedStr<127>;

/// String value up to 256 bytes long.
///
/// NOW-PROTO: NOW_STRING256
pub type NowString256 = NowRestrictedStr<255>;

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("hello", &[0x05, 0x00, 0x00, 0x00, b'h', b'e', b'l', b'l', b'o', 0x00])]
    #[case("", &[0x00, 0x00, 0x00, 0x00, 0x00])]
    fn now_lrgstr_roundtrip(#[case] value: &str, #[case] expected_encoded: &'static [u8]) {
        let mut encoded_value = [0u8; 32];
        let encoded_size = crate::encode(&NowLrgStr::new(value).unwrap(), &mut encoded_value).unwrap();

        assert_eq!(encoded_size, expected_encoded.len());
        assert_eq!(&encoded_value[..encoded_size], expected_encoded);

        let decoded_value = crate::decode::<NowLrgStr>(&encoded_value).unwrap();
        assert_eq!(decoded_value.0, value);
    }

    #[test]
    fn decoded_now_lrgstr_invalid_utf8() {
        let encoded = [0x01, 0x00, 0x00, 0x00, 0xFF, 0x00];
        crate::decode::<NowLrgStr>(&encoded).unwrap_err();
    }

    #[rstest]
    #[case("hello", &[0x05, b'h', b'e', b'l', b'l', b'o', 0x00])]
    #[case("", &[0x00, 0x00])]
    fn now_varstr_roundtrip(#[case] value: &str, #[case] expected_encoded: &'static [u8]) {
        let mut encoded_value = [0u8; 32];
        let encoded_size = crate::encode(&NowVarStr::new(value).unwrap(), &mut encoded_value).unwrap();

        assert_eq!(encoded_size, expected_encoded.len());
        assert_eq!(&encoded_value[..encoded_size], expected_encoded);

        let decoded_value = crate::decode::<NowVarStr>(&encoded_value).unwrap();
        assert_eq!(decoded_value.0, value);
    }

    #[test]
    fn decoded_now_varstr_invalid_utf8() {
        let encoded = [0x01, 0xFF, 0x00];
        crate::decode::<NowLrgStr>(&encoded).unwrap_err();
    }

    #[rstest]
    #[case("hello", &[0x05, b'h', b'e', b'l', b'l', b'o', 0x00])]
    #[case("", &[0x00, 0x00])]
    fn now_restricted_string_roundtrip(#[case] value: &str, #[case] expected_encoded: &'static [u8]) {
        let mut encoded_value = [0u8; 32];
        let encoded_size = crate::encode(&NowString16::new(value).unwrap(), &mut encoded_value).unwrap();

        assert_eq!(encoded_size, expected_encoded.len());
        assert_eq!(&encoded_value[..encoded_size], expected_encoded);

        let decoded_value = crate::decode::<NowString16>(&encoded_value).unwrap();
        assert_eq!(decoded_value.0, value);
    }

    #[test]
    fn now_restricted_string_too_large_constructed() {
        let value = "a".repeat(16);
        NowString16::new(value).unwrap_err();
    }

    #[test]
    fn decoded_now_restricted_string_invalid_utf8() {
        let encoded = [0x01, 0xFF, 0x00];
        crate::decode::<NowString16>(&encoded).unwrap_err();
    }
}
