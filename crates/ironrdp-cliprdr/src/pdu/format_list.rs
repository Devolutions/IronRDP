use std::borrow::Cow;

use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::utils::{read_string_from_cursor, to_utf16_bytes, write_string_to_cursor, CharacterSet};
use ironrdp_pdu::{
    cast_int, ensure_size, impl_pdu_borrowing, impl_pdu_pod, invalid_message_err, IntoOwnedPdu, PduDecode, PduEncode,
    PduResult,
};

use crate::pdu::{ClipboardPduFlags, PartialHeader};

/// Clipboard format id.
///
/// [Standard clipboard formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/standard-clipboard-formats)
/// defined by Microsoft are available as constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClipboardFormatId(u32);

impl ClipboardFormatId {
    /// Text format. Each line ends with a carriage return/linefeed (CR-LF) combination.
    /// A null character signals the end of the data. Use this format for ANSI text.
    pub const CF_TEXT: Self = Self(1);

    /// A handle to a bitmap (HBITMAP).
    pub const CF_BITMAP: Self = Self(2);

    /// Handle to a metafile picture format as defined by the METAFILEPICT structure.
    ///
    /// When passing a CF_METAFILEPICT handle by means of DDE, the application responsible for
    /// deleting hMem should also free the metafile referred to by the CF_METAFILEPICT handle.
    pub const CF_METAFILEPICT: Self = Self(3);

    /// Microsoft Symbolic Link (SYLK) format.
    pub const CF_SYLK: Self = Self(4);

    /// Software Arts' Data Interchange Format.
    pub const CF_DIF: Self = Self(5);

    /// Tagged-image file format.
    pub const CF_TIFF: Self = Self(6);

    /// Text format containing characters in the OEM character set. Each line ends with a carriage
    /// return/linefeed (CR-LF) combination. A null character signals the end of the data.
    pub const CF_OEMTEXT: Self = Self(7);

    /// A memory object containing a BITMAPINFO structure followed by the bitmap bits.
    pub const CF_DIB: Self = Self(8);

    /// Handle to a color palette.
    ///
    /// Whenever an application places data in the clipboard that
    /// depends on or assumes a color palette, it should place the palette on the clipboard as well.
    /// If the clipboard contains data in the CF_PALETTE (logical color palette) format, the
    /// application should use the SelectPalette and RealizePalette functions to realize (compare)
    /// any other data in the clipboard against that logical palette. When displaying clipboard
    /// data, the clipboard always uses as its current palette any object on the clipboard that is
    /// in the CF_PALETTE format.
    ///
    /// NOTE: When transferred over `CLIPRDR`, [`crate::pdu::format_data::ClipboardPalette`] structure
    /// is used instead of `HPALETTE`.
    pub const CF_PALETTE: Self = Self(9);

    /// Data for the pen extensions to the Microsoft Windows for Pen Computing.
    pub const CF_PENDATA: Self = Self(10);

    /// Represents audio data more complex than can be represented in a CF_WAVE standard wave format.
    pub const CF_RIFF: Self = Self(11);

    /// Represents audio data in one of the standard wave formats, such as 11 kHz or 22 kHz PCM.
    pub const CF_WAVE: Self = Self(12);

    /// Unicode text format. Each line ends with a carriage return/linefeed (CR-LF) combination.
    /// A null character signals the end of the data.
    pub const CF_UNICODETEXT: Self = Self(13);

    /// A handle to an enhanced metafile (HENHMETAFILE).
    ///
    /// NOTE: When transferred over `CLIPRDR`, [`crate::pdu::format_data::PackedMetafile`] structure
    /// is used instead of `HENHMETAFILE`.
    pub const CF_ENHMETAFILE: Self = Self(14);

    /// A handle to type HDROP that identifies a list of files. An application can retrieve
    /// information about the files by passing the handle to the DragQueryFile function.
    pub const CF_HDROP: Self = Self(15);

    /// The data is a handle (HGLOBAL) to the locale identifier (LCID) associated with text in the
    /// clipboard.
    ///
    /// When you close the clipboard, if it contains CF_TEXT data but no CF_LOCALE data,
    /// the system automatically sets the CF_LOCALE format to the current input language. You can
    /// use the CF_LOCALE format to associate a different locale with the clipboard text. An
    /// application that pastes text from the clipboard can retrieve this format to determine which
    /// character set was used to generate the text. Note that the clipboard does not support plain
    /// text in multiple character sets. To achieve this, use a formatted text data type such as
    /// RTF instead.The system uses the code page associated with CF_LOCALE to implicitly convert
    /// from CF_TEXT to CF_UNICODETEXT. Therefore, the correct code page table is used for the
    /// conversion.
    pub const CF_LOCALE: Self = Self(16);

    /// A memory object containing a BITMAPV5HEADER structure followed by the bitmap color space
    /// information and the bitmap bits.
    pub const CF_DIBV5: Self = Self(17);

    /// Creates new `ClipboardFormatId` with given id. Note that [`ClipboardFormatId`] already
    /// defines constants for standard clipboard formats, [`Self::new`] should only be
    /// used for custom/OS-specific formats.
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    pub fn value(&self) -> u32 {
        self.0
    }

    pub fn is_standard(self) -> bool {
        matches!(
            self,
            Self::CF_TEXT
                | Self::CF_BITMAP
                | Self::CF_METAFILEPICT
                | Self::CF_SYLK
                | Self::CF_DIF
                | Self::CF_TIFF
                | Self::CF_OEMTEXT
                | Self::CF_DIB
                | Self::CF_PALETTE
                | Self::CF_PENDATA
                | Self::CF_RIFF
                | Self::CF_WAVE
                | Self::CF_UNICODETEXT
                | Self::CF_ENHMETAFILE
                | Self::CF_HDROP
                | Self::CF_LOCALE
                | Self::CF_DIBV5
        )
    }

    pub fn is_registrered(self) -> bool {
        (self.0 >= 0xC000) && (self.0 <= 0xFFFF)
    }
}

/// Clipboard format name. Hardcoded format names defined by [MS-RDPECLIP] are available as
/// constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardFormatName(Cow<'static, str>);

impl ClipboardFormatName {
    /// Special format name for file lists defined by [`MS-RDPECLIP`] which is used for clipboard
    /// data  with [`crate::pdu::format_data::PackedFileList`] payload.
    pub const FILE_LIST: Self = Self::new_static("FileGroupDescriptorW");

    pub fn new(name: impl Into<Cow<'static, str>>) -> Self {
        Self(name.into())
    }

    /// Same as [`Self::new`], but for `'static` string - it can be used in const contexts.
    pub const fn new_static(name: &'static str) -> Self {
        Self(Cow::Borrowed(name))
    }

    pub fn value(&self) -> &str {
        &self.0
    }
}

/// Represents `CLIPRDR_SHORT_FORMAT_NAME` and `CLIPRDR_LONG_FORMAT_NAME`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardFormat {
    id: ClipboardFormatId,
    name: Option<ClipboardFormatName>,
}

impl ClipboardFormat {
    /// Creates unnamed `ClipboardFormat` with given id.
    pub const fn new(id: ClipboardFormatId) -> Self {
        Self { id, name: None }
    }

    /// Sets clipboard format name.
    ///
    /// This is typically used for custom/OS-specific formats where a name must be associated to
    /// the `ClipboardFormatId` in order to distinguish between vendors.
    #[must_use]
    pub fn with_name(self, name: ClipboardFormatName) -> Self {
        if name.0.is_empty() {
            return Self {
                id: self.id,
                name: None,
            };
        }

        Self {
            id: self.id,
            name: Some(name),
        }
    }

    pub fn id(&self) -> ClipboardFormatId {
        self.id
    }

    pub fn name(&self) -> Option<&ClipboardFormatName> {
        self.name.as_ref()
    }
}

/// Represents `CLIPRDR_FORMAT_LIST`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatList<'a> {
    use_ascii: bool,
    encoded_formats: Cow<'a, [u8]>,
}

impl_pdu_borrowing!(FormatList<'_>, OwnedFormatList);

impl IntoOwnedPdu for FormatList<'_> {
    type Owned = OwnedFormatList;

    fn into_owned_pdu(self) -> Self::Owned {
        OwnedFormatList {
            use_ascii: self.use_ascii,
            encoded_formats: Cow::Owned(self.encoded_formats.into_owned()),
        }
    }
}

impl FormatList<'_> {
    const NAME: &str = "CLIPRDR_FORMAT_LIST";

    // `CLIPRDR_SHORT_FORMAT_NAME` size
    const SHORT_FORMAT_SIZE: usize = std::mem::size_of::<u32>() + 32;

    fn new_impl(formats: &[ClipboardFormat], use_long_format: bool, use_ascii: bool) -> PduResult<Self> {
        let charset = if use_ascii {
            CharacterSet::Ansi
        } else {
            CharacterSet::Unicode
        };

        let mut bytes_written = 0;

        if use_long_format {
            // Sane default for formats buffer size to avoid reallocations
            const DEFAULT_STRING_BUFFER_SIZE: usize = 1024;
            let mut buffer = vec![0u8; DEFAULT_STRING_BUFFER_SIZE];

            for format in formats {
                let encoded_string = match charset {
                    CharacterSet::Ansi => {
                        let mut str_buffer = format
                            .name
                            .as_ref()
                            .map(|name| name.value().as_bytes().to_vec())
                            .unwrap_or_default();
                        str_buffer.push(b'\0');
                        str_buffer
                    }
                    CharacterSet::Unicode => {
                        let mut str_buffer = format
                            .name
                            .as_ref()
                            .map(|name| to_utf16_bytes(name.value()))
                            .unwrap_or_default();
                        str_buffer.push(b'\0');
                        str_buffer.push(b'\0');
                        str_buffer
                    }
                };

                let required_size = std::mem::size_of::<u32>() + encoded_string.len();
                if buffer.len() - bytes_written < required_size {
                    buffer.resize(bytes_written + required_size, 0);
                }

                let mut cursor = WriteCursor::new(&mut buffer[bytes_written..]);

                // Write will never fail, as we pre-allocated space in buffer
                cursor.write_u32(format.id.value());
                cursor.write_slice(&encoded_string);

                bytes_written += required_size;
            }

            buffer.truncate(bytes_written);

            Ok(Self {
                use_ascii,
                encoded_formats: Cow::Owned(buffer),
            })
        } else {
            let mut buffer = vec![0u8; Self::SHORT_FORMAT_SIZE * formats.len()];
            for (idx, format) in formats.iter().enumerate() {
                let mut cursor = WriteCursor::new(&mut buffer[idx * Self::SHORT_FORMAT_SIZE..]);
                cursor.write_u32(format.id.value());
                write_string_to_cursor(
                    &mut cursor,
                    format.name.as_ref().map(|name| name.value()).unwrap_or_default(),
                    charset,
                    true,
                )?;
            }

            Ok(Self {
                use_ascii,
                encoded_formats: Cow::Owned(buffer),
            })
        }
    }

    pub fn new_unicode(formats: &[ClipboardFormat], use_long_format: bool) -> PduResult<Self> {
        Self::new_impl(formats, use_long_format, false)
    }

    pub fn new_ascii(formats: &[ClipboardFormat], use_long_format: bool) -> PduResult<Self> {
        Self::new_impl(formats, use_long_format, true)
    }

    pub fn get_formats(&self, use_long_format: bool) -> PduResult<Vec<ClipboardFormat>> {
        let mut src = ReadCursor::new(self.encoded_formats.as_ref());
        let charset = if self.use_ascii {
            CharacterSet::Ansi
        } else {
            CharacterSet::Unicode
        };

        if use_long_format {
            // Minimal `CLIPRDR_LONG_FORMAT_NAME` size (id + null-terminated name)
            const MINIMAL_FORMAT_SIZE: usize = std::mem::size_of::<u32>() + std::mem::size_of::<u16>();

            let mut formats = Vec::with_capacity(16);

            while src.len() >= MINIMAL_FORMAT_SIZE {
                let id = src.read_u32();
                let name = read_string_from_cursor(&mut src, charset, true)?;

                let format = ClipboardFormat::new(ClipboardFormatId::new(id)).with_name(ClipboardFormatName::new(name));

                formats.push(format);
            }

            Ok(formats)
        } else {
            let items_count = src.len() / Self::SHORT_FORMAT_SIZE;

            let mut formats = Vec::with_capacity(items_count);

            for _ in 0..items_count {
                let id = src.read_u32();
                let name_buffer = src.read_slice(32);

                let mut name_cursor: ReadCursor<'_> = ReadCursor::new(name_buffer);
                let name = read_string_from_cursor(&mut name_cursor, charset, true)?;

                let format = ClipboardFormat::new(ClipboardFormatId(id)).with_name(ClipboardFormatName::new(name));

                formats.push(format);
            }

            Ok(formats)
        }
    }
}

impl<'de> PduDecode<'de> for FormatList<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let header = PartialHeader::decode(src)?;

        let use_ascii = header.message_flags.contains(ClipboardPduFlags::ASCII_NAMES);
        ensure_size!(in: src, size: header.data_length());

        let encoded_formats = src.read_slice(header.data_length());

        Ok(Self {
            use_ascii,
            encoded_formats: Cow::Borrowed(encoded_formats),
        })
    }
}

impl PduEncode for FormatList<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header_flags = if self.use_ascii {
            ClipboardPduFlags::ASCII_NAMES
        } else {
            ClipboardPduFlags::empty()
        };

        let header = PartialHeader::new_with_flags(cast_int!("dataLen", self.encoded_formats.len())?, header_flags);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.encoded_formats.len());

        dst.write_slice(&self.encoded_formats);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + self.encoded_formats.len()
    }
}

/// Represents `FORMAT_LIST_RESPONSE`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatListResponse {
    Ok,
    Fail,
}

impl_pdu_pod!(FormatListResponse);

impl FormatListResponse {
    const NAME: &str = "FORMAT_LIST_RESPONSE";
}

impl PduEncode for FormatListResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header_flags = match self {
            FormatListResponse::Ok => ClipboardPduFlags::RESPONSE_OK,
            FormatListResponse::Fail => ClipboardPduFlags::RESPONSE_FAIL,
        };

        let header = PartialHeader::new_with_flags(0, header_flags);
        header.encode(dst)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE
    }
}

impl<'de> PduDecode<'de> for FormatListResponse {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let header = PartialHeader::decode(src)?;
        match header.message_flags {
            ClipboardPduFlags::RESPONSE_OK => Ok(FormatListResponse::Ok),
            ClipboardPduFlags::RESPONSE_FAIL => Ok(FormatListResponse::Fail),
            _ => Err(invalid_message_err!("msgFlags", "Invalid format list message flags")),
        }
    }
}
