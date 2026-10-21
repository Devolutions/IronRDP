//! OS-independent parts of clipboard file transfer: building a file list from local paths,
//! answering `FileContentsRequest`s from it, sizing outgoing `RANGE` requests, and deciding
//! which clipboard formats can travel as plain `HGLOBAL` bytes.
//!
//! Kept apart from the OLE and Win32 plumbing so it can be tested without a desktop session.

use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::os::windows::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, FileContentsFlags,
    FileContentsRequest, FileContentsResponse, FileDescriptor, MAX_FILE_COUNT, PackedFileList,
};
use tracing::debug;

/// Largest chunk answered to, or asked for in, a single `RANGE` request.
///
/// [MS-RDPECLIP] 2.2.5.3 makes `cbRequested` an upper bound on what the responder returns,
/// so capping it here bounds both the allocation when serving a request and the time a
/// single round trip keeps the clipboard thread waiting.
pub const MAX_FILE_CONTENTS_CHUNK: u32 = 1024 * 1024;

/// File positions at or above this offset require `CB_HUGE_FILE_SUPPORT_ENABLED`
/// ([MS-RDPECLIP] 2.2.5.3, `nPositionHigh`).
pub const HUGE_FILE_POSITION: u64 = 0x8000_0000;

/// Longest wire file name, in UTF-16 code units, excluding the null terminator.
///
/// `cFileName` is 260 UTF-16 code units ([MS-RDPECLIP] 2.2.5.2.3.1).
const MAX_WIRE_NAME_LEN: usize = 259;

/// Clipboard formats whose data is not a self-contained `HGLOBAL` byte buffer: it is a GDI or
/// metafile handle, an `IStream`/`IStorage`, or a path or shell item that only exists on the
/// machine it came from. File contents travel through the file stream protocol instead.
const NON_PORTABLE_FORMAT_NAMES: &[&str] = &[
    // Local paths and shell items.
    "FileName",
    "FileNameW",
    "FileNameMap",
    "FileNameMapW",
    "Shell IDList Array",
    "Shell Object Offsets",
    // File transfer, handled by `FileGroupDescriptorW` and the file stream protocol.
    "FileGroupDescriptor",
    "FileGroupDescriptorW",
    "FileContents",
    "Preferred DropEffect",
    "Performed DropEffect",
    "Paste Succeeded",
    // OLE objects backed by `IStorage`/`IStream` or process-local state.
    "DataObject",
    "Ole Private Data",
    "Embed Source",
    "Embedded Object",
    "Link Source",
    "Link Source Descriptor",
    "Object Descriptor",
];

/// A local file or directory offered to the remote, with the descriptor sent for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFileEntry {
    pub descriptor: FileDescriptor,
    pub path: PathBuf,
}

/// Returns whether data in `format` can be exchanged as the raw bytes of an `HGLOBAL`.
///
/// Handle-based formats (`CF_BITMAP`, `CF_ENHMETAFILE`, ...) are not: Windows synthesizes
/// `CF_DIB` from `CF_BITMAP`, so images still go through as DIBs.
pub fn is_portable_format(format: &ClipboardFormat) -> bool {
    const CF_OWNERDISPLAY: u32 = 0x0080;
    const CF_DSPENHMETAFILE: u32 = 0x008E;
    const CF_PRIVATEFIRST: u32 = 0x0200;
    const CF_GDIOBJLAST: u32 = 0x03FF;

    let id = format.id();

    if matches!(
        id,
        ClipboardFormatId::CF_BITMAP
            | ClipboardFormatId::CF_METAFILEPICT
            | ClipboardFormatId::CF_PALETTE
            | ClipboardFormatId::CF_ENHMETAFILE
            | ClipboardFormatId::CF_HDROP
    ) {
        return false;
    }

    // CF_OWNERDISPLAY and the CF_DSP* private display formats
    if (CF_OWNERDISPLAY..=CF_DSPENHMETAFILE).contains(&id.value()) {
        return false;
    }

    // CF_PRIVATEFIRST..CF_PRIVATELAST and CF_GDIOBJFIRST..CF_GDIOBJLAST
    if (CF_PRIVATEFIRST..=CF_GDIOBJLAST).contains(&id.value()) {
        return false;
    }

    !format.name().is_some_and(|name| {
        NON_PORTABLE_FORMAT_NAMES
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(name.value()))
    })
}

/// Returns whether `format` is the file list format, which is identified by name.
pub fn is_file_list_format(format: &ClipboardFormat) -> bool {
    format.name().map(ClipboardFormatName::value) == Some(ClipboardFormatName::FILE_LIST.value())
}

/// Returns whether `Cliprdr::initiate_file_copy` keeps `descriptor` in the file list it sends.
///
/// It drops descriptors with an empty name, a wire name longer than 259 characters, or an
/// absolute wire name. Dropping shifts the index of every later entry, and the remote's
/// `FileContentsRequest`s address files by that index, so lists are filtered here first and
/// stay index-aligned with what is actually sent.
pub fn is_sendable_descriptor(descriptor: &FileDescriptor) -> bool {
    if descriptor.name.is_empty() {
        return false;
    }

    let (first_component, wire_len) = match descriptor.relative_path.as_deref() {
        Some(path) if !path.is_empty() => (
            path,
            path.encode_utf16()
                .count()
                .saturating_add(1)
                .saturating_add(descriptor.name.encode_utf16().count()),
        ),
        _ => (descriptor.name.as_str(), descriptor.name.encode_utf16().count()),
    };

    wire_len <= MAX_WIRE_NAME_LEN && !is_absolute_wire_name(first_component)
}

fn is_absolute_wire_name(name: &str) -> bool {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some('/' | '\\'), _) => true,
        (Some(drive), Some(':')) => drive.is_ascii_alphabetic(),
        _ => false,
    }
}

/// Builds the list of files and directories to offer for the given clipboard paths
/// (`CF_HDROP` entries), walking directories recursively.
///
/// Every directory comes before its contents, so the receiver can create it first, and
/// children are named relative to the copied item (`dir\sub\file.txt`). Entries the remote
/// cannot receive are left out (see [`is_sendable_descriptor`]), as are symbolic links and
/// junctions to directories below the top level, which could otherwise loop forever.
pub fn build_local_file_list(roots: &[PathBuf]) -> Vec<LocalFileEntry> {
    let mut entries = Vec::new();

    for root in roots {
        let Some(name) = root.file_name().and_then(|name| name.to_str()) else {
            debug!(path = %root.display(), "Skipping clipboard path without a usable file name");
            continue;
        };

        collect_entry(root, name, None, &mut entries);
    }

    entries
}

fn collect_entry(path: &Path, name: &str, relative_path: Option<&str>, entries: &mut Vec<LocalFileEntry>) {
    if MAX_FILE_COUNT <= entries.len() {
        debug!(path = %path.display(), "Clipboard file list is full; skipping");
        return;
    }

    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            debug!(%error, path = %path.display(), "Skipping unreadable clipboard file");
            return;
        }
    };

    let mut descriptor = FileDescriptor::new(name)
        .with_attributes(file_attributes(metadata.file_attributes()))
        .with_last_write_time(metadata.last_write_time());
    if let Some(relative_path) = relative_path {
        descriptor = descriptor.with_relative_path(relative_path);
    }
    if !metadata.is_dir() {
        descriptor = descriptor.with_file_size(metadata.len());
    }

    if !is_sendable_descriptor(&descriptor) {
        // Children would have even longer names, so the whole subtree goes.
        debug!(path = %path.display(), "Skipping clipboard file whose name can't be sent");
        return;
    }

    entries.push(LocalFileEntry {
        descriptor,
        path: path.to_path_buf(),
    });

    if !metadata.is_dir() {
        return;
    }

    let children_path = match relative_path {
        Some(relative_path) => format!("{relative_path}\\{name}"),
        None => name.to_owned(),
    };

    let mut children = match fs::read_dir(path) {
        Ok(read_dir) => read_dir.filter_map(Result::ok).collect::<Vec<_>>(),
        Err(error) => {
            debug!(%error, path = %path.display(), "Failed to list clipboard directory");
            return;
        }
    };
    children.sort_by_key(fs::DirEntry::file_name);

    for child in children {
        let child_path = child.path();

        let is_linked_directory = child.file_type().is_ok_and(|file_type| file_type.is_symlink())
            && fs::metadata(&child_path).is_ok_and(|metadata| metadata.is_dir());
        if is_linked_directory {
            debug!(path = %child_path.display(), "Skipping linked directory in clipboard file list");
            continue;
        }

        let file_name = child.file_name();
        let Some(child_name) = file_name.to_str() else {
            debug!(path = %child_path.display(), "Skipping clipboard path without a usable file name");
            continue;
        };

        collect_entry(&child_path, child_name, Some(&children_path), entries);
    }
}

fn file_attributes(raw: u32) -> ClipboardFileAttributes {
    let known = ClipboardFileAttributes::READONLY
        | ClipboardFileAttributes::HIDDEN
        | ClipboardFileAttributes::SYSTEM
        | ClipboardFileAttributes::DIRECTORY
        | ClipboardFileAttributes::ARCHIVE
        | ClipboardFileAttributes::NORMAL;

    let attributes = ClipboardFileAttributes::from_bits_retain(raw) & known;

    if attributes.is_empty() {
        ClipboardFileAttributes::NORMAL
    } else {
        attributes
    }
}

/// Returns whether `descriptor` can be offered when `CB_HUGE_FILE_SUPPORT_ENABLED` was not
/// negotiated: files are then limited to 4,294,967,295 bytes ([MS-RDPECLIP] 2.2.2.1.1.1).
pub fn fits_without_huge_file_support(descriptor: &FileDescriptor) -> bool {
    descriptor
        .file_size
        .is_none_or(|file_size| u32::try_from(file_size).is_ok())
}

/// Returns whether `descriptor` describes a directory, which has no contents to transfer.
pub fn is_directory(descriptor: &FileDescriptor) -> bool {
    descriptor
        .attributes
        .is_some_and(|attributes| attributes.contains(ClipboardFileAttributes::DIRECTORY))
}

/// Answers a `FileContentsRequest` for `entry` from the file on disk.
///
/// `RANGE` reads are capped at [`MAX_FILE_CONTENTS_CHUNK`]; a short answer is valid and the
/// requester asks again for the rest.
pub fn read_file_contents(request: &FileContentsRequest, entry: &LocalFileEntry) -> FileContentsResponse<'static> {
    if is_directory(&entry.descriptor) {
        debug!(index = request.index, "File contents requested for a directory");
        return FileContentsResponse::new_error(request.stream_id);
    }

    if request.flags.contains(FileContentsFlags::SIZE) {
        return match fs::metadata(&entry.path) {
            Ok(metadata) => FileContentsResponse::new_size_response(request.stream_id, metadata.len()),
            Err(error) => {
                debug!(%error, index = request.index, "Failed to read clipboard file size");
                FileContentsResponse::new_error(request.stream_id)
            }
        };
    }

    let read_len = request.requested_size.min(MAX_FILE_CONTENTS_CHUNK);

    let result = fs::File::open(&entry.path).and_then(|mut file| {
        file.seek(SeekFrom::Start(request.position))?;
        let mut data = Vec::new();
        file.take(u64::from(read_len)).read_to_end(&mut data)?;
        Ok(data)
    });

    match result {
        Ok(data) => FileContentsResponse::new_data_response(request.stream_id, data),
        Err(error) => {
            debug!(%error, index = request.index, position = request.position, "Failed to read clipboard file");
            FileContentsResponse::new_error(request.stream_id)
        }
    }
}

/// Size of the `RANGE` request to send for `wanted` bytes at `position`.
///
/// The request is capped at [`MAX_FILE_CONTENTS_CHUNK`] and, when the file size is known,
/// at the bytes left in the file: `Cliprdr::request_file_contents` rejects a range past the
/// end of the file. Returns 0 once there is nothing left to ask for.
pub fn range_request_len(position: u64, file_size: Option<u64>, wanted: u32) -> u32 {
    let wanted = wanted.min(MAX_FILE_CONTENTS_CHUNK);

    match file_size {
        Some(file_size) => {
            let remaining = file_size.saturating_sub(position);
            u32::try_from(remaining).map_or(wanted, |remaining| remaining.min(wanted))
        }
        None => wanted,
    }
}

/// Encodes a file list as a Windows `FILEGROUPDESCRIPTORW`.
///
/// `CLIPRDR_FILELIST` ([MS-RDPECLIP] 2.2.5.2.3) is laid out exactly like
/// `FILEGROUPDESCRIPTORW`, so the PDU encoding is the native format.
pub fn encode_file_group_descriptor(files: &[FileDescriptor]) -> Option<Vec<u8>> {
    let list = PackedFileList { files: files.to_vec() };

    match ironrdp_core::encode_vec(&list) {
        Ok(data) => Some(data),
        Err(error) => {
            debug!(%error, "Failed to encode file group descriptor");
            None
        }
    }
}

/// Decodes a Windows `FILEGROUPDESCRIPTORW`, the inverse of [`encode_file_group_descriptor`].
///
/// File names are kept as found, with any `\`-separated directories still in `name`.
pub fn decode_file_group_descriptor(data: &[u8]) -> Option<Vec<FileDescriptor>> {
    match ironrdp_core::decode::<PackedFileList>(data) {
        Ok(list) => Some(list.files),
        Err(error) => {
            debug!(%error, "Failed to decode file group descriptor");
            None
        }
    }
}
