mod file_list;
mod metafile;
mod palette;

pub use self::file_list::*;
pub use self::metafile::*;
pub use self::palette::*;

#[rustfmt::skip]
use std::borrow::Cow;

use ironrdp_core::cast_int;
use ironrdp_core::ensure_fixed_part_size;
use ironrdp_core::ensure_size;
use ironrdp_core::DecodeResult;
use ironrdp_core::EncodeResult;
use ironrdp_core::IntoOwned;
use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::utils::{read_string_from_cursor, to_utf16_bytes, CharacterSet};
use ironrdp_pdu::{impl_pdu_borrowing, Decode, Encode};

use super::ClipboardFormatId;
use crate::pdu::{ClipboardPduFlags, PartialHeader};

/// Represents `CLIPRDR_FORMAT_DATA_RESPONSE`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDataResponse<'a> {
    is_error: bool,
    data: Cow<'a, [u8]>,
}

impl_pdu_borrowing!(FormatDataResponse<'_>, OwnedFormatDataResponse);

impl IntoOwned for FormatDataResponse<'_> {
    type Owned = OwnedFormatDataResponse;

    fn into_owned(self) -> Self::Owned {
        OwnedFormatDataResponse {
            is_error: self.is_error,
            data: Cow::Owned(self.data.into_owned()),
        }
    }
}

impl<'a> FormatDataResponse<'a> {
    const NAME: &'static str = "CLIPRDR_FORMAT_DATA_RESPONSE";

    /// Creates new format data response from raw data.
    pub fn new_data(data: impl Into<Cow<'a, [u8]>>) -> Self {
        Self {
            is_error: false,
            data: data.into(),
        }
    }

    /// Creates new error format data response.
    pub fn new_error() -> Self {
        Self {
            is_error: true,
            data: Cow::Borrowed(&[]),
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn is_error(&self) -> bool {
        self.is_error
    }

    /// Creates new format data response from clipboard palette. Please note that this method
    /// allocates memory for the data automatically. If you want to avoid this, you can use
    /// `new_data` method and encode [`ClipboardPalette`] prior to the call.
    pub fn new_palette(palette: &ClipboardPalette) -> EncodeResult<Self> {
        let mut data = vec![0u8; palette.size()];

        let mut cursor = WriteCursor::new(&mut data);
        palette.encode(&mut cursor)?;

        Ok(Self {
            is_error: false,
            data: data.into(),
        })
    }

    /// Creates new format data response from packed metafile. Please note that this method
    /// allocates memory for the data automatically. If you want to avoid this, you can use
    /// `new_data` method and encode [`PackedMetafile`] prior to the call.
    pub fn new_metafile(metafile: &PackedMetafile<'_>) -> EncodeResult<Self> {
        let mut data = vec![0u8; metafile.size()];

        let mut cursor = WriteCursor::new(&mut data);
        metafile.encode(&mut cursor)?;

        Ok(Self {
            is_error: false,
            data: data.into(),
        })
    }

    /// Creates new format data response from packed file list. Please note that this method
    /// allocates memory for the data automatically. If you want to avoid this, you can use
    /// `new_data` method and encode [`PackedFileList`] prior to the call.
    pub fn new_file_list(list: &PackedFileList) -> EncodeResult<Self> {
        let mut data = vec![0u8; list.size()];

        let mut cursor = WriteCursor::new(&mut data);
        list.encode(&mut cursor)?;

        Ok(Self {
            is_error: false,
            data: data.into(),
        })
    }

    /// Creates new format data response from string.
    pub fn new_unicode_string(value: &str) -> Self {
        let mut encoded = to_utf16_bytes(value);
        encoded.push(b'\0');
        encoded.push(b'\0');

        Self {
            is_error: false,
            data: encoded.into(),
        }
    }

    /// Creates new format data response from string.
    pub fn new_string(value: &str) -> Self {
        let mut encoded = value.as_bytes().to_vec();
        encoded.push(b'\0');

        Self {
            is_error: false,
            data: encoded.into(),
        }
    }

    /// Reads inner data as [`ClipboardPalette`]
    pub fn to_palette(&self) -> DecodeResult<ClipboardPalette> {
        let mut cursor = ReadCursor::new(&self.data);
        ClipboardPalette::decode(&mut cursor)
    }

    /// Reads inner data as [`PackedMetafile`]
    pub fn to_metafile(&self) -> DecodeResult<PackedMetafile<'_>> {
        let mut cursor = ReadCursor::new(&self.data);
        PackedMetafile::decode(&mut cursor)
    }

    /// Reads inner data as [`PackedFileList`]
    pub fn to_file_list(&self) -> DecodeResult<PackedFileList> {
        let mut cursor = ReadCursor::new(&self.data);
        PackedFileList::decode(&mut cursor)
    }

    /// Reads inner data as string
    pub fn to_string(&self) -> DecodeResult<String> {
        let mut cursor = ReadCursor::new(&self.data);
        read_string_from_cursor(&mut cursor, CharacterSet::Ansi, true)
    }

    /// Reads inner data as unicode string
    pub fn to_unicode_string(&self) -> DecodeResult<String> {
        let mut cursor = ReadCursor::new(&self.data);
        read_string_from_cursor(&mut cursor, CharacterSet::Unicode, true)
    }

    pub fn into_data(self) -> Cow<'a, [u8]> {
        self.data
    }
}

impl Encode for FormatDataResponse<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let flags = if self.is_error {
            ClipboardPduFlags::RESPONSE_FAIL
        } else {
            ClipboardPduFlags::RESPONSE_OK
        };

        let header = PartialHeader::new_with_flags(cast_int!("dataLen", self.data.len())?, flags);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.data.len());
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + self.data.len()
    }
}

impl<'de> Decode<'de> for FormatDataResponse<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = PartialHeader::decode(src)?;

        let is_error = header.message_flags.contains(ClipboardPduFlags::RESPONSE_FAIL);

        ensure_size!(in: src, size: header.data_length());
        let data = src.read_slice(header.data_length());

        Ok(Self {
            is_error,
            data: Cow::Borrowed(data),
        })
    }
}

/// Represents `CLIPRDR_FORMAT_DATA_REQUEST`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDataRequest {
    pub format: ClipboardFormatId,
}

impl FormatDataRequest {
    const NAME: &'static str = "CLIPRDR_FORMAT_DATA_REQUEST";
    const FIXED_PART_SIZE: usize = 4 /* format */;
}

impl Encode for FormatDataRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let header = PartialHeader::new(cast_int!("dataLen", Self::FIXED_PART_SIZE)?);
        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.format.value());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for FormatDataRequest {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_fixed_part_size!(in: src);
        let format = ClipboardFormatId::new(src.read_u32());

        Ok(Self { format })
    }
}
