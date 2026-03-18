use std::borrow::Cow;

use bitflags::bitflags;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
};
use ironrdp_pdu::utils::{CharacterSet, combine_u64, decode_string, encode_string, split_u64};
use ironrdp_pdu::{impl_pdu_pod, write_padding};

/// Maximum file name field size in bytes (260 UTF-16 code units * 2 bytes per code unit).
const NAME_LENGTH: usize = 520;

/// Defense-in-depth limit on the number of file descriptors in a single
/// [`PackedFileList`]. This prevents memory exhaustion from crafted payloads
/// while remaining well above any realistic file transfer count.
pub const MAX_FILE_COUNT: usize = 100_000;

bitflags! {
    /// Represents `flags` field of `CLIPRDR_FILEDESCRIPTOR` structure.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClipboardFileFlags: u32 {
        /// The fileAttributes field contains valid data.
        const ATTRIBUTES = 0x0000_0004;
        /// The lastWriteTime field contains valid data.
        const LAST_WRITE_TIME = 0x0000_0020;
        /// The fileSizeHigh and fileSizeLow fields contain valid data.
        const FILE_SIZE = 0x0000_0040;
        /// A progress indicator should be shown when copying the file.
        const SHOW_PROGRESS_UI = 0x0000_4000;

        const _ = !0;
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

        const _ = !0;
    }
}

/// [2.2.5.2.3.1] File Descriptor (CLIPRDR_FILEDESCRIPTOR)
///
/// The `name` field holds the file basename (e.g., `"file.txt"`).
/// The `relative_path` field holds the directory portion of the path
/// (e.g., `"temp\\subdir"`), using `\` as the separator to match
/// the Windows convention on the wire. `None` means the file is at
/// the root level of the copied collection.
///
/// Per [MS-RDPECLIP] 3.1.1.2, file lists use relative paths to describe
/// directory structure (e.g., `temp\file1.txt`). The sanitization layer
/// in [`crate::Cliprdr`] populates both fields from the raw wire name.
///
/// # Encoding constraints
///
/// The wire `cFileName` field is 520 bytes (260 UTF-16 code units).
/// [`Encode::encode`] will return an error if the reconstructed wire name
/// (`relative_path` + `\` + `name`) exceeds this limit. Callers that
/// construct descriptors directly (rather than via decode) must ensure
/// the combined name fits.
///
/// [2.2.5.2.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/a765d784-2b39-4b88-9faa-88f8666f9c35
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileDescriptor {
    pub attributes: Option<ClipboardFileAttributes>,
    pub last_write_time: Option<u64>,
    pub file_size: Option<u64>,
    // TODO: Define a new type for "bounded" strings (this one should never be bigger than 260 characters, including the null-terminator)
    pub name: String,
    /// Relative directory path for this file within the copied collection.
    /// Uses `\` as the separator. `None` for root-level files.
    pub relative_path: Option<String>,
}

impl_pdu_pod!(FileDescriptor);

impl FileDescriptor {
    const NAME: &'static str = "CLIPRDR_FILEDESCRIPTOR";

    /// Creates a new file descriptor with the given name and all optional fields set to `None`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            attributes: None,
            last_write_time: None,
            file_size: None,
            name: name.into(),
            relative_path: None,
        }
    }

    /// Sets the file attributes.
    #[must_use]
    pub fn with_attributes(mut self, attributes: ClipboardFileAttributes) -> Self {
        self.attributes = Some(attributes);
        self
    }

    /// Sets the last write time (Windows FILETIME).
    #[must_use]
    pub fn with_last_write_time(mut self, time: u64) -> Self {
        self.last_write_time = Some(time);
        self
    }

    /// Sets the file size in bytes.
    #[must_use]
    pub fn with_file_size(mut self, size: u64) -> Self {
        self.file_size = Some(size);
        self
    }

    /// Sets the relative directory path within the copied collection.
    #[must_use]
    pub fn with_relative_path(mut self, path: impl Into<String>) -> Self {
        self.relative_path = Some(path.into());
        self
    }

    const FIXED_PART_SIZE: usize = 4 /* dwFlags */
        + 32 /* reserved1 */
        + 4 /* dwFileAttributes */
        + 16 /* reserved2 */
        + 8 /* ftLastWriteTime */
        + 8 /* nFileSizeHigh + nFileSizeLow */
        + NAME_LENGTH /* cFileName */;

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

        // Reconstruct the wire fileName from relative_path and name.
        // Per MS-RDPECLIP 3.1.1.2, file lists use relative paths like "temp\file1.txt".
        let wire_name: Cow<'_, str> = match &self.relative_path {
            Some(path) if !path.is_empty() => Cow::Owned(format!("{path}\\{}", self.name)),
            _ => Cow::Borrowed(&self.name),
        };

        // Validate length before writing to prevent buffer corruption when
        // encoding multiple descriptors into a shared buffer.
        // UTF-16 encoding: each code unit is 2 bytes, plus 2 bytes for null terminator.
        let encoded_len = wire_name.encode_utf16().count() * 2 + 2;
        if NAME_LENGTH < encoded_len {
            return Err(ironrdp_core::invalid_field_err!(
                "cFileName",
                "encoded wire name exceeds NAME_LENGTH (520 bytes)"
            ));
        }

        let written = encode_string(dst.remaining_mut(), &wire_name, CharacterSet::Unicode, true)?;
        dst.advance(written);

        // Pad with zeroes, overriding any previously written data
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

        let flags = ClipboardFileFlags::from_bits_retain(src.read_u32());
        src.read_array::<32>();
        let attributes = if flags.contains(ClipboardFileFlags::ATTRIBUTES) {
            Some(ClipboardFileAttributes::from_bits_retain(src.read_u32()))
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

        // Bound the scan to exactly the 520-byte cFileName field so that
        // malformed data (missing null terminator) cannot read into
        // subsequent file descriptors.
        let name_field = &src.remaining()[..NAME_LENGTH];
        let name = decode_string(name_field, CharacterSet::Unicode, true)?;
        src.advance(NAME_LENGTH);

        Ok(Self {
            attributes,
            last_write_time,
            file_size,
            name,
            // Populated later by the sanitization layer in Cliprdr::process()
            relative_path: None,
        })
    }
}

/// [2.2.5.2.3] Packed File List (CLIPRDR_FILELIST)
///
/// [2.2.5.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/a1db10b8-4a2a-4ce4-8e5f-6ce5bec5c979
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedFileList {
    pub files: Vec<FileDescriptor>,
}

impl_pdu_pod!(PackedFileList);

impl PackedFileList {
    const NAME: &'static str = "CLIPRDR_FILELIST";
    const FIXED_PART_SIZE: usize = 4 /* cItems */;
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
        let file_count: usize = cast_length!(Self::NAME, "cItems", src.read_u32())?;

        if MAX_FILE_COUNT < file_count {
            return Err(ironrdp_core::invalid_field_err!(
                "cItems",
                "file count exceeds maximum of 100000"
            ));
        }

        // Cap pre-allocation against remaining bytes to prevent OOM from
        // a malicious file_count. The actual decode loop will fail gracefully
        // if the cursor runs out of data.
        let max_possible = src.len() / FileDescriptor::SIZE;
        let mut files = Vec::with_capacity(file_count.min(max_possible));
        for _ in 0..file_count {
            files.push(FileDescriptor::decode(src)?);
        }

        Ok(Self { files })
    }
}
