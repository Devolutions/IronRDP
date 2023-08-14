use crate::cursor::{ReadCursor, WriteCursor};
use crate::utils::{read_string_from_cursor, write_string_to_cursor, CharacterSet};
use crate::{ensure_fixed_part_size, invalid_message_err, PduDecode, PduEncode, PduError, PduErrorKind, PduResult};
use bitflags::bitflags;
use std::borrow::Cow;

struct PartialHeader {
    pub message_flags: ClipboardPduFlags,
    pub data_length: u32,
}

impl PartialHeader {
    const NAME: &str = "CLIPRDR_HEADER";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>() + std::mem::size_of::<u32>();
    const SIZE: usize = Self::FIXED_PART_SIZE;

    pub fn new(inner_data_length: u32) -> Self {
        Self::new_with_flags(inner_data_length, ClipboardPduFlags::empty())
    }

    pub fn new_with_flags(data_length: u32, message_flags: ClipboardPduFlags) -> Self {
        Self {
            message_flags,
            data_length,
        }
    }

    pub fn inner_data_length(&self) -> usize {
        self.data_length as usize
    }
}

impl<'de> PduDecode<'de> for PartialHeader {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let message_flags = ClipboardPduFlags::from_bits_truncate(src.read_u16());
        let data_length = src.read_u32();

        Ok(Self {
            message_flags,
            data_length,
        })
    }
}

impl PduEncode for PartialHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.message_flags.bits());
        dst.write_u32(self.data_length);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

const MSG_TYPE_MONITOR_READY: u16 = 0x0001;
const MSG_TYPE_FORMAT_LIST: u16 = 0x0002;
const MSG_TYPE_FORMAT_LIST_RESPONSE: u16 = 0x0003;
const MSG_TYPE_FORMAT_DATA_REQUEST: u16 = 0x0004;
const MSG_TYPE_FORMAT_DATA_RESPONSE: u16 = 0x0005;
const MSG_TYPE_TEMPORARY_DIRECTORY: u16 = 0x0006;
const MSG_TYPE_CAPABILITIES: u16 = 0x0007;
const MSG_TYPE_FILE_CONTENTS_REQUEST: u16 = 0x0008;
const MSG_TYPE_FILE_CONTENTS_RESPONSE: u16 = 0x0009;
const MSG_TYPE_LOCK_CLIPDATA: u16 = 0x000A;
const MSG_TYPE_UNLOCK_CLIPDATA: u16 = 0x000B;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardPdu<'a> {
    MonitorReady,
    FormatList(FormatList<'a>),
    FormatListResponse(FormatListResponse),
    FormatDataRequest(FormatDataRequest),
    FormatDataResponse(FormatDataResponse<'a>),
    TemporaryDirectory(ClipboardClientTemporaryDirectory<'a>),
    Capabilites(ClipboardCapabilities),
    FileContentsRequest(FileContentsRequest),
    FileContentsResponse(FileContentsResponse<'a>),
    LockData(ClipboardLockDataId),
    UnlockData(ClipboardLockDataId),
}

impl ClipboardPdu<'_> {
    const NAME: &str = "CliboardPdu";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>();
}

impl PduEncode for ClipboardPdu<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let write_empty_pdu = |dst: &mut WriteCursor<'_>| {
            let header = PartialHeader::new(0);
            header.encode(dst)
        };

        match self {
            ClipboardPdu::MonitorReady => {
                dst.write_u16(MSG_TYPE_MONITOR_READY);
                write_empty_pdu(dst)
            }
            ClipboardPdu::FormatList(pdu) => {
                dst.write_u16(MSG_TYPE_FORMAT_LIST);
                pdu.encode(dst)
            }
            ClipboardPdu::FormatListResponse(pdu) => {
                dst.write_u16(MSG_TYPE_FORMAT_LIST_RESPONSE);
                pdu.encode(dst)
            }
            ClipboardPdu::FormatDataRequest(pdu) => {
                dst.write_u16(MSG_TYPE_FORMAT_DATA_REQUEST);
                pdu.encode(dst)
            }
            ClipboardPdu::FormatDataResponse(pdu) => {
                dst.write_u16(MSG_TYPE_FORMAT_DATA_RESPONSE);
                pdu.encode(dst)
            }
            ClipboardPdu::TemporaryDirectory(pdu) => {
                dst.write_u16(MSG_TYPE_TEMPORARY_DIRECTORY);
                pdu.encode(dst)
            }
            ClipboardPdu::Capabilites(pdu) => {
                dst.write_u16(MSG_TYPE_CAPABILITIES);
                pdu.encode(dst)
            }
            ClipboardPdu::FileContentsRequest(pdu) => {
                dst.write_u16(MSG_TYPE_FILE_CONTENTS_REQUEST);
                pdu.encode(dst)
            }
            ClipboardPdu::FileContentsResponse(pdu) => {
                dst.write_u16(MSG_TYPE_FILE_CONTENTS_RESPONSE);
                pdu.encode(dst)
            }
            ClipboardPdu::LockData(pdu) => {
                dst.write_u16(MSG_TYPE_LOCK_CLIPDATA);
                pdu.encode(dst)
            }
            ClipboardPdu::UnlockData(pdu) => {
                dst.write_u16(MSG_TYPE_UNLOCK_CLIPDATA);
                pdu.encode(dst)
            }
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let empty_size = PartialHeader::SIZE;

        let variable_size = match self {
            ClipboardPdu::MonitorReady => empty_size,
            ClipboardPdu::FormatList(pdu) => pdu.size(),
            ClipboardPdu::FormatListResponse(pdu) => pdu.size(),
            ClipboardPdu::FormatDataRequest(pdu) => pdu.size(),
            ClipboardPdu::FormatDataResponse(pdu) => pdu.size(),
            ClipboardPdu::TemporaryDirectory(pdu) => pdu.size(),
            ClipboardPdu::Capabilites(pdu) => pdu.size(),
            ClipboardPdu::FileContentsRequest(pdu) => pdu.size(),
            ClipboardPdu::FileContentsResponse(pdu) => pdu.size(),
            ClipboardPdu::LockData(pdu) => pdu.size(),
            ClipboardPdu::UnlockData(pdu) => pdu.size(),
        };

        Self::FIXED_PART_SIZE + variable_size
    }
}

impl<'de> PduDecode<'de> for ClipboardPdu<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let read_empty_pdu = |src: &mut ReadCursor<'de>| {
            let _header = PartialHeader::decode(src)?;
            Ok(())
        };

        let pdu = match src.read_u16() {
            MSG_TYPE_MONITOR_READY => {
                read_empty_pdu(src)?;
                ClipboardPdu::MonitorReady
            }
            MSG_TYPE_FORMAT_LIST => ClipboardPdu::FormatList(FormatList::decode(src)?),
            MSG_TYPE_FORMAT_LIST_RESPONSE => ClipboardPdu::FormatListResponse(FormatListResponse::decode(src)?),
            MSG_TYPE_FORMAT_DATA_REQUEST => ClipboardPdu::FormatDataRequest(FormatDataRequest::decode(src)?),
            MSG_TYPE_FORMAT_DATA_RESPONSE => ClipboardPdu::FormatDataResponse(FormatDataResponse::decode(src)?),
            MSG_TYPE_TEMPORARY_DIRECTORY => {
                ClipboardPdu::TemporaryDirectory(ClipboardClientTemporaryDirectory::decode(src)?)
            }
            MSG_TYPE_CAPABILITIES => ClipboardPdu::Capabilites(ClipboardCapabilities::decode(src)?),
            MSG_TYPE_FILE_CONTENTS_REQUEST => ClipboardPdu::FileContentsRequest(FileContentsRequest::decode(src)?),
            MSG_TYPE_FILE_CONTENTS_RESPONSE => ClipboardPdu::FileContentsResponse(FileContentsResponse::decode(src)?),
            MSG_TYPE_LOCK_CLIPDATA => ClipboardPdu::LockData(ClipboardLockDataId::decode(src)?),
            MSG_TYPE_UNLOCK_CLIPDATA => ClipboardPdu::UnlockData(ClipboardLockDataId::decode(src)?),
            _ => return Err(invalid_message_err!("msgType", "Unknown clipboard PDU type")),
        };

        Ok(pdu)
    }
}

/// Represents `CLIPRDR_FILEDESCRIPTOR`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDescriptor {
    pub attibutes: Option<ClipboardFileAttributes>,
    pub last_write_time: Option<u64>,
    pub file_size: Option<u64>,
    pub name: String,
}

impl FileDescriptor {
    const NAME: &str = "CLIPRDR_FILEDESCRIPTOR";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>() // flags
        + 32 // reserved
        + std::mem::size_of::<u32>() // attributes
        + 16 // reserved
        + std::mem::size_of::<u64>() // last write time
        + std::mem::size_of::<u64>() // size
        + 520; // name

    const SIZE: usize = Self::FIXED_PART_SIZE;
}

impl PduEncode for FileDescriptor {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let mut flags = ClipboardFileFlags::empty();
        if self.attibutes.is_some() {
            flags |= ClipboardFileFlags::ATTRIBUTES;
        }
        if self.last_write_time.is_some() {
            flags |= ClipboardFileFlags::LAST_WRITE_TIME;
        }
        if self.file_size.is_some() {
            flags |= ClipboardFileFlags::FILE_SIZE;
        }

        dst.write_u32(flags.bits());
        dst.write_array([0u8; 32]);
        dst.write_u32(self.attibutes.unwrap_or(ClipboardFileAttributes::empty()).bits());
        dst.write_array([0u8; 16]);
        dst.write_u64(self.last_write_time.unwrap_or_default());

        let (size_lo, size_hi) = split_u64(self.file_size.unwrap_or_default());
        dst.write_u32(size_hi);
        dst.write_u32(size_lo);

        {
            let mut cursor = WriteCursor::new(dst.remaining_mut());
            write_string_to_cursor(&mut cursor, &self.name, CharacterSet::Unicode, true)?;
        }

        dst.advance(520);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for FileDescriptor {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = ClipboardFileFlags::from_bits_truncate(src.read_u32());
        src.read_array::<32>();
        let attibutes = if flags.contains(ClipboardFileFlags::ATTRIBUTES) {
            Some(ClipboardFileAttributes::from_bits_truncate(src.read_u32()))
        } else {
            let _ = src.read_u32();
            None
        };
        src.read_array::<16>();
        let last_write_time = if flags.contains(ClipboardFileFlags::LAST_WRITE_TIME) {
            Some(src.read_u64())
        } else {
            let _ = src.read_u64();
            None
        };
        let file_size = if flags.contains(ClipboardFileFlags::FILE_SIZE) {
            let size_hi = src.read_u32();
            let size_lo = src.read_u32();
            Some(combine_u64(size_lo, size_hi))
        } else {
            let _ = src.read_u64();
            None
        };

        let name = {
            let mut cursor = ReadCursor::new(src.remaining());
            read_string_from_cursor(&mut cursor, CharacterSet::Unicode, true)?
        };

        src.advance(520);

        Ok(Self {
            attibutes,
            last_write_time,
            file_size,
            name,
        })
    }
}

/// Represents `CLIPRDR_FILELIST`
///
/// NOTE: `PduDecode` implementation will read all remaining data in cursor as file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedFileList {
    pub files: Vec<FileDescriptor>,
}

impl PackedFileList {
    const NAME: &str = "CLIPRDR_FILELIST";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>(); // file count
}

impl PduEncode for PackedFileList {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.files.len() as u32);

        for file in &self.files {
            file.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + FileDescriptor::SIZE * self.files.len()
    }
}

impl<'de> PduDecode<'de> for PackedFileList {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let file_count = src.read_u32() as usize;

        let mut files = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            files.push(FileDescriptor::decode(src)?);
        }

        Ok(Self { files })
    }
}

/// Represents `CLIPRDR_MFPICT`
///
/// NOTE: `PduDecode` implementation will read all remaining data in cursor as metafile contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedMetafile<'a> {
    pub mapping_mode: PackedMetafileMappingMode,
    pub x_ext: u32,
    pub y_ext: u32,
    /// The variable sized contents of the metafile as specified in [MS-WMF] section 2
    pub data: Cow<'a, [u8]>,
}

impl PackedMetafile<'_> {
    const NAME: &str = "CLIPRDR_MFPICT";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>() * 3;
}

impl PduEncode for PackedMetafile<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.mapping_mode.bits());
        dst.write_u32(self.x_ext);
        dst.write_u32(self.y_ext);
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data.len()
    }
}

impl<'de> PduDecode<'de> for PackedMetafile<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let mapping_mode = PackedMetafileMappingMode::from_bits_truncate(src.read_u32());
        let x_ext = src.read_u32();
        let y_ext = src.read_u32();

        let data_len = src.len();

        let data = src.read_slice(data_len);

        Ok(Self {
            mapping_mode,
            x_ext,
            y_ext,
            data: Cow::Borrowed(data),
        })
    }
}

/// Represents `PALETTEENTRY`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub extra: u8,
}

impl PaletteEntry {
    const SIZE: usize = std::mem::size_of::<u8>() * 4;
}

/// Represents `CLIPRDR_PALETTE`
///
/// NOTE: `PduDecode` implementation will read all remaining data in cursor as the palette entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPalette {
    pub entries: Vec<PaletteEntry>,
}

impl ClipboardPalette {
    const NAME: &str = "CLIPRDR_PALETTE";
}

impl PduEncode for ClipboardPalette {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        for entry in &self.entries {
            dst.write_u8(entry.red);
            dst.write_u8(entry.green);
            dst.write_u8(entry.blue);
            dst.write_u8(entry.extra);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.entries.len() * PaletteEntry::SIZE
    }
}

impl<'de> PduDecode<'de> for ClipboardPalette {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let entries_count = src.len() / PaletteEntry::SIZE;

        let mut entries = Vec::with_capacity(entries_count);
        for _ in 0..entries_count {
            let red = src.read_u8();
            let green = src.read_u8();
            let blue = src.read_u8();
            let extra = src.read_u8();

            entries.push(PaletteEntry {
                red,
                green,
                blue,
                extra,
            });
        }

        Ok(Self { entries })
    }
}

/// Represents `CLIPRDR_FORMAT_DATA_RESPONSE`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDataResponse<'a> {
    is_error: bool,
    data: Cow<'a, [u8]>,
}

impl<'a> FormatDataResponse<'a> {
    const NAME: &str = "CLIPRDR_FORMAT_DATA_RESPONSE";

    pub fn new_with_data(data: impl Into<Cow<'a, [u8]>>) -> Self {
        Self {
            is_error: false,
            data: data.into(),
        }
    }

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
    /// `with_data` method and encode [`ClipboardPalette`] prior to the call.
    pub fn new_palette(palette: &ClipboardPalette) -> PduResult<Self> {
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
    /// `with_data` method and encode [`PackedMetafile`] prior to the call.
    pub fn new_packed_metafile(metafile: &PackedMetafile) -> PduResult<Self> {
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
    /// `with_data` method and encode [`PackedFileList`] prior to the call.
    pub fn new_packed_file_list(list: &PackedFileList) -> PduResult<Self> {
        let mut data = vec![0u8; list.size()];

        let mut cursor = WriteCursor::new(&mut data);
        list.encode(&mut cursor)?;

        Ok(Self {
            is_error: false,
            data: data.into(),
        })
    }

    /// Reads inner data as [`ClipboardPalette`]
    pub fn to_palette(&self) -> PduResult<ClipboardPalette> {
        let mut cursor = ReadCursor::new(&self.data);
        ClipboardPalette::decode(&mut cursor)
    }

    /// Reads inner data as [`PackedMetafile`]
    pub fn to_packed_metafile(&self) -> PduResult<PackedMetafile> {
        let mut cursor = ReadCursor::new(&self.data);
        PackedMetafile::decode(&mut cursor)
    }

    /// Reads inner data as [`PackedFileList`]
    pub fn to_packed_file_list(&self) -> PduResult<PackedFileList> {
        let mut cursor = ReadCursor::new(&self.data);
        PackedFileList::decode(&mut cursor)
    }
}

impl PduEncode for FormatDataResponse<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let flags = if self.is_error {
            ClipboardPduFlags::RESPONSE_FAIL
        } else {
            ClipboardPduFlags::RESPONSE_OK
        };

        let header = PartialHeader::new_with_flags(self.data.len() as u32, flags);
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

impl<'de> PduDecode<'de> for FormatDataResponse<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let header = PartialHeader::decode(src)?;

        let is_error = header.message_flags.contains(ClipboardPduFlags::RESPONSE_FAIL);

        ensure_size!(in: src, size: header.inner_data_length());
        let data = src.read_slice(header.inner_data_length());

        Ok(Self {
            is_error,
            data: Cow::Borrowed(data),
        })
    }
}

/// Represents `CLIPRDR_FORMAT_DATA_REQUEST`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDataRequest {
    pub format_id: u32,
}

impl FormatDataRequest {
    const NAME: &str = "CLIPRDR_FORMAT_DATA_REQUEST";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>();
}

impl PduEncode for FormatDataRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = PartialHeader::new(Self::FIXED_PART_SIZE as u32);
        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.format_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for FormatDataRequest {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_fixed_part_size!(in: src);
        let format_id = src.read_u32();

        Ok(Self { format_id })
    }
}

pub const FORMAT_ID_PALETTE: u32 = 9;
pub const FORMAT_ID_METAFILE: u32 = 3;
pub const FILE_LIST_FORMAT_NAME: &str = "FileGroupDescriptorW";

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
                let approximate_item_length = std::mem::size_of::<u32>()
                    + match charset {
                        CharacterSet::Ansi => format.name.len() + 1,
                        CharacterSet::Unicode => {
                            // This is only an approximation, but it's enough to avoid reallocations in
                            // most cases. If resulting string is bigger than that, we will resize the
                            // buffer later when first encoding attempt fails.
                            (format.name.len() + 1) * std::mem::size_of::<u16>()
                        }
                    };

                buffer.resize(buffer.len() + approximate_item_length, 0);

                let mut cursor = WriteCursor::new(&mut buffer[bytes_written..]);

                // Format id write will never fail, as we pre-allocated space in buffer
                cursor.write_u32(format.id);

                let encoding_attempt = write_string_to_cursor(&mut cursor, &format.name, charset, true);

                match encoding_attempt {
                    Ok(()) => {
                        bytes_written += cursor.pos();
                    }
                    Err(PduError {
                        kind: PduErrorKind::NotEnoughBytes { received, expected },
                        ..
                    }) => {
                        // Re-allocate buffer, try again. `write_string_to_cursor` will return
                        // `PduErrorKind::NotEnoughBytes` error if buffer is too small
                        buffer.resize(buffer.len() + expected - received, 0);

                        // Keep written format id in buffer
                        bytes_written += std::mem::size_of::<u32>();

                        let mut cursor = WriteCursor::new(&mut buffer[bytes_written..]);
                        write_string_to_cursor(&mut cursor, &format.name, charset, true)?;
                        bytes_written += cursor.pos();
                    }
                    Err(e) => return Err(e),
                }
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
        ensure_size!(in: src, size: header.inner_data_length());

        let encoded_formats = src.read_slice(header.inner_data_length());

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

/// Represents `CLIPRDR_FILECONTENTS_RESPONSE`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContentsResponse<'a> {
    is_error: bool,
    stream_id: u32,
    data: Cow<'a, [u8]>,
}

impl<'a> FileContentsResponse<'a> {
    const NAME: &str = "CLIPRDR_FILECONTENTS_RESPONSE";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>();

    fn inner_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data.len()
    }

    pub fn new_size_response(stream_id: u32, size: u64) -> Self {
        Self {
            is_error: false,
            stream_id,
            data: Cow::Owned(size.to_le_bytes().to_vec()),
        }
    }

    pub fn new_data_response(stream_id: u32, data: &'a [u8]) -> Self {
        Self {
            is_error: false,
            stream_id,
            data: Cow::Borrowed(data),
        }
    }

    pub fn new_error(stream_id: u32) -> Self {
        Self {
            is_error: true,
            stream_id,
            data: Cow::Borrowed(&[]),
        }
    }

    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_as_size(&self) -> PduResult<u64> {
        if self.data.len() != 8 {
            return Err(invalid_message_err!(
                "requestedFileContentsData",
                "Invalid data size for u64 size"
            ));
        }

        Ok(u64::from_le_bytes(self.data.as_ref().try_into().unwrap()))
    }
}

impl PduEncode for FileContentsResponse<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let flags = if self.is_error {
            ClipboardPduFlags::RESPONSE_FAIL
        } else {
            ClipboardPduFlags::RESPONSE_OK
        };

        let header = PartialHeader::new_with_flags(self.inner_size() as u32, flags);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.inner_size());

        dst.write_u32(self.stream_id);
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + self.inner_size()
    }
}

impl<'de> PduDecode<'de> for FileContentsResponse<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let header = PartialHeader::decode(src)?;

        let is_error = header.message_flags.contains(ClipboardPduFlags::RESPONSE_FAIL);

        ensure_size!(in: src, size: header.inner_data_length());

        let data_size = header.inner_data_length() - Self::FIXED_PART_SIZE;

        let stream_id = src.read_u32();
        let data = src.read_slice(data_size);

        Ok(Self {
            is_error,
            stream_id,
            data: Cow::Borrowed(data),
        })
    }
}

/// Represents `CLIPRDR_FILECONTENTS_REQUEST`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContentsRequest {
    pub stream_id: u32,
    pub index: u32,
    pub flags: FileContentsFlags,
    pub position: u64,
    pub requested_size: u32,
    pub data_id: Option<u32>,
}

impl FileContentsRequest {
    const NAME: &str = "CLIPRDR_FILECONTENTS_REQUEST";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>() * 4 + std::mem::size_of::<u64>();

    fn inner_size(&self) -> usize {
        let data_id_size = match self.data_id {
            Some(_) => std::mem::size_of::<u32>(),
            None => 0,
        };

        Self::FIXED_PART_SIZE + data_id_size
    }
}

impl PduEncode for FileContentsRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = PartialHeader::new(self.inner_size() as u32);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.inner_size());

        dst.write_u32(self.stream_id);
        dst.write_u32(self.index);
        dst.write_u32(self.flags.bits());

        let (position_lo, position_hi) = split_u64(self.position);
        dst.write_u32(position_lo);
        dst.write_u32(position_hi);
        dst.write_u32(self.requested_size);

        if let Some(data_id) = self.data_id {
            dst.write_u32(data_id);
        };

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + self.inner_size()
    }
}

impl<'de> PduDecode<'de> for FileContentsRequest {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let header = PartialHeader::decode(src)?;

        let read_data_id = header.inner_data_length() > Self::FIXED_PART_SIZE;

        let mut expected_size = Self::FIXED_PART_SIZE;
        if read_data_id {
            expected_size += std::mem::size_of::<u32>();
        }

        ensure_size!(in: src, size: expected_size);

        let stream_id = src.read_u32();
        let index = src.read_u32();
        let flags = FileContentsFlags::from_bits_truncate(src.read_u32());
        let position_lo = src.read_u32();
        let position_hi = src.read_u32();
        let position = combine_u64(position_lo, position_hi);
        let requested_size = src.read_u32();
        let data_id = if read_data_id { Some(src.read_u32()) } else { None };

        Ok(Self {
            stream_id,
            index,
            flags,
            position,
            requested_size,
            data_id,
        })
    }
}

/// Represents `CLIPRDR_LOCK_CLIPDATA`/`CLIPRDR_UNLOCK_CLIPDATA`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardLockDataId(pub u32);

impl ClipboardLockDataId {
    const NAME: &str = "CLIPRDR_(UN)LOCK_CLIPDATA";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>();
}

impl PduEncode for ClipboardLockDataId {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = PartialHeader::new(Self::FIXED_PART_SIZE as u32);
        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.0);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for ClipboardLockDataId {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_fixed_part_size!(in: src);
        let id = src.read_u32();

        Ok(Self(id))
    }
}

/// Represents `CLIPRDR_TEMP_DIRECTORY`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardClientTemporaryDirectory<'a> {
    path_buffer: Cow<'a, [u8]>,
}

impl ClipboardClientTemporaryDirectory<'_> {
    const PATH_BUFFER_SIZE: usize = 520;

    const NAME: &str = "CLIPRDR_TEMP_DIRECTORY";
    const FIXED_PART_SIZE: usize = Self::PATH_BUFFER_SIZE;

    pub fn new(path: String) -> PduResult<Self> {
        let mut buffer = vec![0x00; Self::PATH_BUFFER_SIZE];

        {
            let mut cursor = WriteCursor::new(&mut buffer);
            write_string_to_cursor(&mut cursor, &path, CharacterSet::Unicode, true)?;
        }

        Ok(Self {
            path_buffer: Cow::Owned(buffer),
        })
    }

    pub fn temporary_directory_path(&self) -> PduResult<String> {
        let mut cursor = ReadCursor::new(&self.path_buffer);

        read_string_from_cursor(&mut cursor, CharacterSet::Unicode, true)
            .map_err(|_| invalid_message_err!("wszTempDir", "failed to decode temp dir path"))
    }

    fn inner_size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl PduEncode for ClipboardClientTemporaryDirectory<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = PartialHeader::new(self.inner_size() as u32);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.inner_size());
        dst.write_slice(&self.path_buffer);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + self.inner_size()
    }
}

impl<'de> PduDecode<'de> for ClipboardClientTemporaryDirectory<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_fixed_part_size!(in: src);
        let buffer = src.read_slice(Self::PATH_BUFFER_SIZE);

        Ok(Self {
            path_buffer: Cow::Borrowed(buffer),
        })
    }
}

/// Represents `CLIPRDR_CAPS`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardCapabilities {
    pub capabilities: Vec<CapabilitySet>,
}

impl ClipboardCapabilities {
    const NAME: &str = "CLIPRDR_CAPS";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>() * 2;

    fn inner_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.capabilities.iter().map(|c| c.size()).sum::<usize>()
    }
}

impl PduEncode for ClipboardCapabilities {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        if self.capabilities.len() > u16::MAX as usize {
            return Err(invalid_message_err!(
                "cCapabilitiesSets",
                "Too much capability sets specified",
            ));
        }

        let header = PartialHeader::new(self.inner_size() as u32);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.inner_size());

        dst.write_u16(self.capabilities.len() as u16);
        dst.write_u16(0); // pad

        for capability in &self.capabilities {
            capability.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.inner_size() + PartialHeader::SIZE
    }
}

impl<'de> PduDecode<'de> for ClipboardCapabilities {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_fixed_part_size!(in: src);
        let capabilities_count = src.read_u16();
        src.read_u16(); // pad

        let mut capabilities = Vec::with_capacity(capabilities_count as usize);

        for _ in 0..capabilities_count {
            let caps = CapabilitySet::decode(src)?;
            capabilities.push(caps);
        }

        Ok(Self { capabilities })
    }
}

/// Represents `CLIPRDR_CAPS_SET`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySet {
    General(GeneralCapabilitySet),
}

impl CapabilitySet {
    const NAME: &str = "CLIPRDR_CAPS_SET";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>() * 2;

    const CAPSTYPE_GENERAL: u16 = 0x0001;
}

impl From<GeneralCapabilitySet> for CapabilitySet {
    fn from(value: GeneralCapabilitySet) -> Self {
        Self::General(value)
    }
}

impl PduEncode for CapabilitySet {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let (caps, length) = match self {
            Self::General(value) => {
                let length = value.size() + Self::FIXED_PART_SIZE;
                (value, length)
            }
        };

        ensure_size!(in: dst, size: length);
        dst.write_u16(Self::CAPSTYPE_GENERAL);
        dst.write_u16(length as u16);
        caps.encode(dst)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let variable_size = match self {
            Self::General(value) => value.size(),
        };

        Self::FIXED_PART_SIZE + variable_size
    }
}

impl<'de> PduDecode<'de> for CapabilitySet {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let caps_type = src.read_u16();
        let _length = src.read_u16();

        match caps_type {
            Self::CAPSTYPE_GENERAL => {
                let general = GeneralCapabilitySet::decode(src)?;
                Ok(Self::General(general))
            }
            _ => Err(invalid_message_err!(
                "capabilitySetType",
                "invalid clipboard capability set type"
            )),
        }
    }
}

/// Represents `CLIPRDR_GENERAL_CAPABILITY` without header
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralCapabilitySet {
    pub version: ClipboardProtocolVersion,
    pub general_flags: ClipboardGeneralCapabilityFlags,
}

impl GeneralCapabilitySet {
    const NAME: &str = "CLIPRDR_GENERAL_CAPABILITY";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>() * 2;
}

impl PduEncode for GeneralCapabilitySet {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.version.into());
        dst.write_u32(self.general_flags.bits());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for GeneralCapabilitySet {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let version: ClipboardProtocolVersion = src.read_u32().try_into()?;
        let general_flags = ClipboardGeneralCapabilityFlags::from_bits_truncate(src.read_u32());

        Ok(Self { version, general_flags })
    }
}

/// Specifies the `Remote Desktop Protocol: Clipboard Virtual Channel Extension` version number.
/// This field is for informational purposes and MUST NOT be used to make protocol capability
/// decisions. The actual features supported are specified via [`ClipboardGeneralCapabilityFlags`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardProtocolVersion {
    V1,
    V2,
}

impl ClipboardProtocolVersion {
    const VERSION_VALUE_V1: u32 = 0x00000001;
    const VERSION_VALUE_V2: u32 = 0x00000002;

    const NAME: &str = "CLIPRDR_CAPS_VERSION";
}

impl From<ClipboardProtocolVersion> for u32 {
    fn from(version: ClipboardProtocolVersion) -> Self {
        match version {
            ClipboardProtocolVersion::V1 => ClipboardProtocolVersion::VERSION_VALUE_V1,
            ClipboardProtocolVersion::V2 => ClipboardProtocolVersion::VERSION_VALUE_V2,
        }
    }
}

impl TryFrom<u32> for ClipboardProtocolVersion {
    type Error = crate::PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            Self::VERSION_VALUE_V1 => Ok(Self::V1),
            Self::VERSION_VALUE_V2 => Ok(Self::V2),
            _ => Err(invalid_message_err!(
                "version",
                "Invalid clipboard capabilities version"
            )),
        }
    }
}

fn split_u64(value: u64) -> (u32, u32) {
    let bytes = value.to_le_bytes();
    let (low, high) = bytes.split_at(std::mem::size_of::<u32>());
    (
        u32::from_le_bytes(low.try_into().unwrap()),
        u32::from_le_bytes(high.try_into().unwrap()),
    )
}

fn combine_u64(lo: u32, hi: u32) -> u64 {
    let mut position_bytes = [0u8; std::mem::size_of::<u64>()];
    position_bytes[..std::mem::size_of::<u32>()].copy_from_slice(&lo.to_le_bytes());
    position_bytes[std::mem::size_of::<u32>()..].copy_from_slice(&hi.to_le_bytes());
    u64::from_le_bytes(position_bytes)
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClipboardGeneralCapabilityFlags: u32 {
        /// The Long Format Name variant of the Format List PDU is supported
        /// for exchanging updated format names. If this flag is not set, the
        /// Short Format Name variant MUST be used. If this flag is set by both
        /// protocol endpoints, then the Long Format Name variant MUST be
        /// used.
        const USE_LONG_FORMAT_NAMES = 0x00000002;
        /// File copy and paste using stream-based operations are supported
        /// using the File Contents Request PDU and File Contents Response
        /// PDU.
        const STREAM_FILECLIP_ENABLED = 0x00000004;
        /// Indicates that any description of files to copy and paste MUST NOT
        /// include the source path of the files.
        const FILECLIP_NO_FILE_PATHS = 0x00000008;
        /// Locking and unlocking of File Stream data on the clipboard is
        /// supported using the Lock Clipboard Data PDU and Unlock Clipboard
        /// Data PDU.
        const CAN_LOCK_CLIPDATA = 0x00000010;
        /// Indicates support for transferring files that are larger than
        /// 4,294,967,295 bytes in size. If this flag is not set, then only files of
        /// size less than or equal to 4,294,967,295 bytes can be exchanged
        /// using the File Contents Request PDU and File Contents
        /// Response PDU.
        const HUGE_FILE_SUPPORT_ENABLED = 0x00000020;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClipboardPduFlags: u16 {
        /// Used by the Format List Response PDU, Format Data Response PDU, and File
        /// Contents Response PDU to indicate that the associated request Format List PDU,
        /// Format Data Request PDU, and File Contents Request PDU were processed
        /// successfully
        const RESPONSE_OK = 0x0001;
        /// Used by the Format List Response PDU, Format Data Response PDU, and File
        /// Contents Response PDU to indicate that the associated Format List PDU, Format
        /// Data Request PDU, and File Contents Request PDU were not processed successfull
        const RESPONSE_FAIL = 0x0002;
        /// Used by the Short Format Name variant of the Format List Response PDU to indicate
        /// that the format names are in ASCII 8
        const ASCII_NAMES = 0x0004;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct FileContentsFlags: u32 {
        /// A request for the size of the file identified by the lindex field. The size MUST be
        /// returned as a 64-bit, unsigned integer. The cbRequested field MUST be set to
        /// 0x00000008 and both the nPositionLow and nPositionHigh fields MUST be
        /// set to 0x00000000.
        const SIZE = 0x00000001;
        /// A request for the data present in the file identified by the lindex field. The data
        /// to be retrieved is extracted starting from the offset given by the nPositionLow
        /// and nPositionHigh fields. The maximum number of bytes to extract is specified
        /// by the cbRequested field.
        const DATA = 0x00000002;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PackedMetafileMappingMode: u32 {
        /// Each logical unit is mapped to one device pixel. Positive x is to the right; positive
        /// y is down.
        const TEXT = 0x00000001;
        /// Each logical unit is mapped to 0.1 millimeter. Positive x is to the right; positive
        /// y is up.
        const LO_METRIC = 0x00000002;
        /// Each logical unit is mapped to 0.01 millimeter. Positive x is to the right; positive
        /// y is up.
        const HI_METRIC = 0x00000003;
        /// Each logical unit is mapped to 0.01 inch. Positive x is to the right; positive y is up.
        const LO_ENGLISH = 0x00000004;
        /// Each logical unit is mapped to 0.001 inch. Positive x is to the right; positive y is up.
        const HI_ENGLISH = 0x00000005;
        /// Each logical unit is mapped to 1/20 of a printer's point (1/1440 of an inch), also
        /// called a twip. Positive x is to the right; positive y is up.
        const TWIPS = 0x00000006;
        /// Logical units are mapped to arbitrary units with equally scaled axes; one unit along
        /// the x-axis is equal to one unit along the y-axis.
        const ISOTROPIC = 0x00000007;
        /// Logical units are mapped to arbitrary units with arbitrarily scaled axes.
        const ANISOTROPIC = 0x00000008;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClipboardFileFlags: u32 {
        /// The fileAttributes field contains valid data.
        const ATTRIBUTES = 0x00000004;
        /// The fileSizeHigh and fileSizeLow fields contain valid data.
        const FILE_SIZE = 0x00000040;
        /// The lastWriteTime field contains valid data.
        const LAST_WRITE_TIME = 0x00000020;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClipboardFileAttributes: u32 {
        /// A file that is read-only. Applications can read the file, but cannot write to
        /// it or delete it
        const READONLY = 0x00000001;
        /// The file or directory is hidden. It is not included in an ordinary directory
        /// listing.
        const HIDDEN = 0x00000002;
        /// A file or directory that the operating system uses a part of, or uses
        /// exclusively.
        const SYSTEM = 0x00000004;
        /// Identifies a directory.
        const DIRECTORY = 0x00000010;
        /// A file or directory that is an archive file or directory. Applications typically
        /// use this attribute to mark files for backup or removal
        const ARCHIVE = 0x00000020;
        /// A file that does not have other attributes set. This attribute is valid only
        /// when used alone.
        const NORMAL = 0x00000080;
    }
}
