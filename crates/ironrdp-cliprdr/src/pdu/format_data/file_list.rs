use bitflags::bitflags;
use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::utils::{combine_u64, read_string_from_cursor, split_u64, write_string_to_cursor, CharacterSet};
use ironrdp_pdu::{cast_length, ensure_fixed_part_size, PduDecode, PduEncode, PduResult};

bitflags! {
    /// Represents `flags` field of `CLIPRDR_FILEDESCRIPTOR` structure.
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
    /// Represents `fileAttributes` of `CLIPRDR_FILEDESCRIPTOR` strucutre.
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

        dst.write_u32(cast_length!(Self::NAME, "cItems", self.files.len())?);

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
