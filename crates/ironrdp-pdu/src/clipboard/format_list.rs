use crate::clipboard::{ClipboardPduFlags, PartialHeader};
use crate::cursor::{ReadCursor, WriteCursor};
use crate::utils::{read_string_from_cursor, to_utf16_bytes, write_string_to_cursor, CharacterSet};
use crate::{invalid_message_err, PduDecode, PduEncode, PduResult};
use std::borrow::Cow;

/// Represents `CLIPRDR_SHORT_FORMAT_NAME` and `CLIPRDR_LONG_FORMAT_NAME`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardFormat {
    pub id: u32,
    pub name: String,
}

/// Represents `CLIPRDR_FORMAT_LIST`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatList<'a> {
    use_ascii: bool,
    encoded_formats: Cow<'a, [u8]>,
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
                        let mut str_buffer = format.name.as_bytes().to_vec();
                        str_buffer.push(b'\0');
                        str_buffer
                    }
                    CharacterSet::Unicode => {
                        let mut str_buffer = to_utf16_bytes(&format.name);
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
                cursor.write_u32(format.id);
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
                cursor.write_u32(format.id);
                write_string_to_cursor(&mut cursor, &format.name, charset, true)?;
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

                formats.push(ClipboardFormat { id, name });
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

                formats.push(ClipboardFormat { id, name });
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

        let header = PartialHeader::new_with_flags(self.encoded_formats.len() as u32, header_flags);
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
