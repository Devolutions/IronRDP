//! Plug and Play Devices Virtual Channel Extension PDUs defined by [MS-RDPEPNP].
//!
//! [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/

use alloc::string::String;
use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, invalid_field_err,
};

pub mod info;
pub mod io;

/// GUID in the packet representation of [MS-DTYP] section 2.3.4.2 "GUID--Packet Representation".
///
/// [MS-DTYP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-dtyp/
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    const NAME: &'static str = "GUID";

    const FIXED_PART_SIZE: usize = 4 /* Data1 */ + 2 /* Data2 */ + 2 /* Data3 */ + 8 /* Data4 */;

    pub const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self {
            data1,
            data2,
            data3,
            data4,
        }
    }
}

impl Encode for Guid {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.data1);
        dst.write_u16(self.data2);
        dst.write_u16(self.data3);
        dst.write_array(self.data4);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for Guid {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self {
            data1: src.read_u32(),
            data2: src.read_u16(),
            data3: src.read_u16(),
            data4: src.read_array(),
        })
    }
}

/// HRESULT value ([MS-ERREF] section 2.1 "HRESULT") carried in the reply messages of the PNP Device I/O Subprotocol.
///
/// [MS-ERREF]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HResult(pub u32);

impl HResult {
    pub const S_OK: Self = Self(0x0000_0000);
    pub const E_ABORT: Self = Self(0x8000_4004);
    pub const E_FAIL: Self = Self(0x8000_4005);

    /// `ERROR_FILE_NOT_FOUND` as an HRESULT.
    pub const FILE_NOT_FOUND: Self = Self::from_win32(0x0002);

    /// `ERROR_INSUFFICIENT_BUFFER` as an HRESULT.
    pub const INSUFFICIENT_BUFFER: Self = Self::from_win32(0x007A);

    /// Maps a Win32 error code to an HRESULT as `HRESULT_FROM_WIN32` does ([MS-ERREF] section 2.1.2).
    ///
    /// [MS-ERREF]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/
    pub const fn from_win32(code: u32) -> Self {
        if code == 0 {
            Self::S_OK
        } else {
            Self(0x8007_0000 | (code & 0xFFFF))
        }
    }

    /// Returns whether the severity bit is clear.
    pub const fn is_success(self) -> bool {
        self.0 & 0x8000_0000 == 0
    }
}

fn utf16_size(value: &str) -> usize {
    value.encode_utf16().count() * 2
}

fn write_utf16(dst: &mut WriteCursor<'_>, value: &str) {
    for unit in value.encode_utf16() {
        dst.write_u16(unit);
    }
}

fn read_utf16_units(field: &'static str, bytes: &[u8]) -> DecodeResult<Vec<u16>> {
    if !bytes.len().is_multiple_of(2) {
        return Err(invalid_field_err!(field, "UTF-16 field has an odd byte length"));
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .collect())
}

fn string_from_utf16(field: &'static str, units: &[u16]) -> DecodeResult<String> {
    String::from_utf16(units).map_err(|_| invalid_field_err!(field, "invalid UTF-16 string"))
}
