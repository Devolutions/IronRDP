use bitflags::bitflags;
use ironrdp_core::{
    cast_length, ensure_fixed_part_size, Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor,
};
use ironrdp_pdu::utils::{combine_u64, decode_string, encode_string, split_u64, CharacterSet};
use ironrdp_pdu::{impl_pdu_pod, write_padding};

const NAME_LENGTH: usize = 520;

bitflags! {
    /// Represents `flags` field of `CLIPRDR_FILEDESCRIPTOR` structure.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClipboardFileFlags: u32 {
        /// The fileAttributes field contains valid data.
        const ATTRIBUTES = 0x0000_0004;
        /// The fileSizeHigh and fileSizeLow fields contain valid data.
        const FILE_SIZE = 0x0000_0040;
        /// The lastWriteTime field contains valid data.
        const LAST_WRITE_TIME = 0x0000_0020;
    }
}

bitflags! {
    /// Represents `fileAttributes` of `CLIPRDR_FILEDESCRIPTOR` structure.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClipboardFileAttributes: u32 {
        /// A file that is read-only. Applications can read the file, but cannot write to
        /// it or delete it
        const READONLY = 0x0000_0001;
        /// The file or directory is hidden. It is not included in an ordinary directory
        /// listing.
        const HIDDEN = 0x0000_0002;
        /// A file or directory that the operating system uses a part of, or uses
        /// exclusively.
        const SYSTEM = 0x0000_0004;
        /// Identifies a directory.
        const DIRECTORY = 0x0000_0010;
        /// A file or directory that is an archive file or directory. Applications typically
        /// use this attribute to mark files for backup or removal
        const ARCHIVE = 0x0000_0020;
        /// A file that does not have other attributes set. This attribute is valid only
        /// when used alone.
        const NORMAL = 0x0000_0080;
    }
}

/// [2.2.5.2.3.1] File Descriptor (CLIPRDR_FILEDESCRIPTOR)
///
/// [2.2.5.2.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/a765d784-2b39-4b88-9faa-88f8666f9c35
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDescriptor {
    pub attributes: Option<ClipboardFileAttributes>,
    pub last_write_time: Option<u64>,
    pub file_size: Option<u64>,
    // TODO: Define a new type for "bounded" strings (this one should never be bigger than 260 characters, including the null-terminator)
    pub name: String,
}

impl_pdu_pod!(FileDescriptor);

impl FileDescriptor {
    const NAME: &'static str = "CLIPRDR_FILEDESCRIPTOR";

    const FIXED_PART_SIZE: usize = 4 // flags
        + 32 // reserved
        + 4 // attributes
        + 16 // reserved
        + 8 // last write time
        + 8 // size
        + NAME_LENGTH; // name

    const SIZE: usize = Self::FIXED_PART_SIZE;
}

impl Encode for FileDescriptor {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        let mut flags = ClipboardFileFlags::empty();
        if self.attributes.is_some() {
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
        dst.write_u32(self.attributes.unwrap_or(ClipboardFileAttributes::empty()).bits());
        dst.write_array([0u8; 16]);
        dst.write_u64(self.last_write_time.unwrap_or_default());

        let (size_lo, size_hi) = split_u64(self.file_size.unwrap_or_default());
        dst.write_u32(size_hi);
        dst.write_u32(size_lo);

        let written = encode_string(dst.remaining_mut(), &self.name, CharacterSet::Unicode, true)?;
        dst.advance(written);

        // Pad with zeroes, overidding any previously written data
        write_padding!(dst, NAME_LENGTH - written);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for FileDescriptor {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = ClipboardFileFlags::from_bits_truncate(src.read_u32());
        src.read_array::<32>();
        let attributes = if flags.contains(ClipboardFileFlags::ATTRIBUTES) {
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

        let name = decode_string(src.remaining(), CharacterSet::Unicode, true)?;
        src.advance(NAME_LENGTH);

        Ok(Self {
            attributes,
            last_write_time,
            file_size,
            name,
        })
    }
}

/// Represents `CLIPRDR_FILELIST`
///
/// NOTE: `Decode` implementation will read all remaining data in cursor as file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedFileList {
    pub files: Vec<FileDescriptor>,
}

impl_pdu_pod!(PackedFileList);

impl PackedFileList {
    const NAME: &'static str = "CLIPRDR_FILELIST";
    const FIXED_PART_SIZE: usize = 4; // file count
}

impl Encode for PackedFileList {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
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

impl<'de> Decode<'de> for PackedFileList {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let file_count = cast_length!(Self::NAME, "cItems", src.read_u32())?;

        let mut files = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            files.push(FileDescriptor::decode(src)?);
        }

        Ok(Self { files })
    }
}
