//! Tests for the parts of the Windows clipboard backend's file transfer that need no desktop
//! session: format filtering, file list building, and file contents requests.

use std::fs;
use std::path::PathBuf;

use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName,
    ClipboardGeneralCapabilityFlags, ClipboardProtocolVersion, FileContentsFlags, FileContentsRequest, FileDescriptor,
};
use ironrdp_cliprdr::{CliprdrClient, CliprdrState};
use ironrdp_cliprdr_native::StubClipboard;
use ironrdp_cliprdr_native::windows_file_list::{
    LocalFileEntry, MAX_FILE_CONTENTS_CHUNK, build_local_file_list, decode_file_group_descriptor,
    encode_file_group_descriptor, fits_without_huge_file_support, is_directory, is_file_list_format,
    is_portable_format, is_sendable_descriptor, range_request_len, read_file_contents,
};

use super::TempDir;

fn named(name: &'static str) -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::new(0xC123)).with_name(ClipboardFormatName::new_static(name))
}

fn request(flags: FileContentsFlags, position: u64, requested_size: u32) -> FileContentsRequest {
    FileContentsRequest {
        stream_id: 7,
        index: 0,
        flags,
        position,
        requested_size,
        data_id: None,
    }
}

fn file_entry(path: PathBuf) -> LocalFileEntry {
    let size = fs::metadata(&path).unwrap().len();
    LocalFileEntry {
        descriptor: FileDescriptor::new("file.bin").with_file_size(size),
        path,
    }
}

#[test]
fn handle_based_formats_are_not_portable() {
    for id in [
        ClipboardFormatId::CF_BITMAP,
        ClipboardFormatId::CF_METAFILEPICT,
        ClipboardFormatId::CF_PALETTE,
        ClipboardFormatId::CF_ENHMETAFILE,
        ClipboardFormatId::CF_HDROP,
        // CF_OWNERDISPLAY, CF_DSPBITMAP, CF_DSPENHMETAFILE
        ClipboardFormatId::new(0x0080),
        ClipboardFormatId::new(0x0082),
        ClipboardFormatId::new(0x008E),
        // CF_PRIVATEFIRST, CF_GDIOBJLAST
        ClipboardFormatId::new(0x0200),
        ClipboardFormatId::new(0x03FF),
    ] {
        assert!(!is_portable_format(&ClipboardFormat::new(id)), "{id:?}");
    }
}

#[test]
fn byte_formats_are_portable() {
    for id in [
        ClipboardFormatId::CF_TEXT,
        ClipboardFormatId::CF_UNICODETEXT,
        ClipboardFormatId::CF_DIB,
        ClipboardFormatId::CF_DIBV5,
        ClipboardFormatId::CF_LOCALE,
    ] {
        assert!(is_portable_format(&ClipboardFormat::new(id)), "{id:?}");
    }

    assert!(is_portable_format(&named("HTML Format")));
    assert!(is_portable_format(&named("Rich Text Format")));
    assert!(is_portable_format(&named("PNG")));
}

#[test]
fn path_and_file_transfer_formats_are_not_portable() {
    for name in [
        "FileNameW",
        "FileName",
        "Shell IDList Array",
        "FileGroupDescriptorW",
        "FileContents",
        "Preferred DropEffect",
        "Ole Private Data",
        // Clipboard format names are case-insensitive.
        "filecontents",
    ] {
        assert!(!is_portable_format(&named(name)), "{name}");
    }
}

#[test]
fn file_list_format_is_found_by_name() {
    assert!(is_file_list_format(&named("FileGroupDescriptorW")));
    assert!(!is_file_list_format(&named("FileGroupDescriptor")));
    assert!(!is_file_list_format(&ClipboardFormat::new(ClipboardFormatId::CF_HDROP)));
}

#[test]
fn sendable_descriptors_match_cliprdr_validation() {
    assert!(is_sendable_descriptor(&FileDescriptor::new("file.txt")));
    assert!(is_sendable_descriptor(
        &FileDescriptor::new("file.txt").with_relative_path("dir\\sub")
    ));
    assert!(is_sendable_descriptor(&FileDescriptor::new("a".repeat(259))));

    assert!(!is_sendable_descriptor(&FileDescriptor::new("")));
    assert!(!is_sendable_descriptor(&FileDescriptor::new("a".repeat(260))));
    // `dir\` + 255 characters is 259: fine. One more is too long.
    assert!(is_sendable_descriptor(
        &FileDescriptor::new("a".repeat(255)).with_relative_path("dir")
    ));
    assert!(!is_sendable_descriptor(
        &FileDescriptor::new("a".repeat(256)).with_relative_path("dir")
    ));
    // Absolute names leak local paths, and `Cliprdr` drops them under CB_FILECLIP_NO_FILE_PATHS.
    assert!(!is_sendable_descriptor(&FileDescriptor::new("C:\\secret.txt")));
    assert!(!is_sendable_descriptor(&FileDescriptor::new("\\\\server\\share")));
    assert!(!is_sendable_descriptor(
        &FileDescriptor::new("file.txt").with_relative_path("C:\\dir")
    ));
}

/// Descriptors at and around each limit `Cliprdr::initiate_file_copy` enforces.
fn edge_descriptors() -> Vec<FileDescriptor> {
    // Two UTF-16 code units, one char: where counting chars and code units disagree.
    let astral = '\u{1F600}';

    let mut descriptors = vec![
        FileDescriptor::new(""),
        FileDescriptor::new("a"),
        FileDescriptor::new("C:\\secret.txt"),
        FileDescriptor::new("C:relative.txt"),
        FileDescriptor::new("\\\\server\\share"),
        FileDescriptor::new("\\rooted.txt"),
        FileDescriptor::new("/etc/passwd"),
        FileDescriptor::new("file.txt").with_relative_path("D:\\dir"),
        FileDescriptor::new("file.txt").with_relative_path("dir\\sub"),
        FileDescriptor::new("file.txt").with_relative_path(""),
    ];

    for len in 257..=261 {
        descriptors.push(FileDescriptor::new("a".repeat(len)));
        descriptors.push(FileDescriptor::new(astral.to_string().repeat(len / 2)));
        descriptors.push(FileDescriptor::new(format!("{}{}", astral, "a".repeat(len - 2))));
        descriptors.push(FileDescriptor::new("b".repeat(len - 101)).with_relative_path("d".repeat(100)));
        descriptors.push(FileDescriptor::new("b".repeat(len - 101)).with_relative_path(astral.to_string().repeat(50)));
    }

    descriptors
}

#[test]
fn sendable_descriptors_are_kept_by_cliprdr() {
    // Indexes of the files offered must match those of the list `Cliprdr` sends, so every
    // descriptor the backend keeps must also be kept by `Cliprdr`, whatever was negotiated.
    for flags in [
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED,
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS,
    ] {
        for descriptor in edge_descriptors().into_iter().filter(is_sendable_descriptor) {
            let mut cliprdr = CliprdrClient::new(StubClipboard::new().backend_factory().build_cliprdr_backend());
            *cliprdr.__test_state_mut() = CliprdrState::Ready;
            // `Cliprdr` always negotiates long format names.
            let flags = flags | ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES;
            *cliprdr.__test_capabilities_mut() = Capabilities::new(ClipboardProtocolVersion::V2, flags);

            let _ = cliprdr.initiate_file_copy(vec![descriptor.clone()]).unwrap();

            let kept = cliprdr
                .__test_local_file_list()
                .as_ref()
                .map_or(0, |list| list.files.len());
            assert_eq!(kept, 1, "dropped by Cliprdr with {flags:?}: {descriptor:?}");

            // And it fits the wire format.
            assert!(encode_file_group_descriptor(&[descriptor]).is_some());
        }
    }

    // The edge cases include both sides of each limit.
    let descriptors = edge_descriptors();
    let sendable = descriptors
        .iter()
        .filter(|descriptor| is_sendable_descriptor(descriptor))
        .count();
    assert!(0 < sendable && sendable < descriptors.len());
}

#[test]
fn file_list_walks_directories_with_relative_paths() {
    let temp = TempDir::new();
    let root = temp.path().join("root");
    fs::create_dir_all(root.join("sub").join("deeper")).unwrap();
    fs::create_dir(root.join("empty")).unwrap();
    fs::write(root.join("a.txt"), b"hello").unwrap();
    fs::write(root.join("sub").join("b.bin"), [0u8; 1000]).unwrap();
    fs::write(root.join("sub").join("deeper").join("c.txt"), b"").unwrap();
    let top_file = temp.path().join("top.txt");
    fs::write(&top_file, b"top").unwrap();

    let entries = build_local_file_list(&[root.clone(), top_file.clone()]);

    let listed = entries
        .iter()
        .map(|entry| {
            let directory = is_directory(&entry.descriptor);
            (
                entry.descriptor.relative_path.as_deref(),
                entry.descriptor.name.as_str(),
                directory,
                entry.descriptor.file_size,
            )
        })
        .collect::<Vec<_>>();

    // Directories come before their contents; children are sorted by name.
    assert_eq!(
        listed,
        [
            (None, "root", true, None),
            (Some("root"), "a.txt", false, Some(5)),
            (Some("root"), "empty", true, None),
            (Some("root"), "sub", true, None),
            (Some("root\\sub"), "b.bin", false, Some(1000)),
            (Some("root\\sub"), "deeper", true, None),
            (Some("root\\sub\\deeper"), "c.txt", false, Some(0)),
            (None, "top.txt", false, Some(3)),
        ]
    );

    assert_eq!(entries[4].path, root.join("sub").join("b.bin"));
    assert_eq!(entries[7].path, top_file);
    assert!(entries.iter().all(|entry| entry.descriptor.last_write_time.is_some()));
}

#[test]
fn file_list_skips_missing_paths_and_names_too_long_to_send() {
    let temp = TempDir::new();
    let long_name = "n".repeat(250);
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    // `root\` + 250 characters fits, but its child doesn't: `root\nnn…\child.txt` is 265.
    fs::create_dir(root.join(&long_name)).unwrap();
    fs::write(root.join(&long_name).join("child.txt"), b"x").unwrap();

    let entries = build_local_file_list(&[temp.path().join("missing"), root]);

    let names = entries
        .iter()
        .map(|entry| entry.descriptor.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["root", long_name.as_str()]);
    assert!(entries.iter().all(|entry| is_sendable_descriptor(&entry.descriptor)));
}

#[test]
fn file_contents_size_and_range() {
    let temp = TempDir::new();
    let path = temp.path().join("file.bin");
    let contents = (0..=255u8).cycle().take(10_000).collect::<Vec<_>>();
    fs::write(&path, &contents).unwrap();
    let entry = file_entry(path);

    let response = read_file_contents(&request(FileContentsFlags::SIZE, 0, 8), &entry);
    assert!(!response.is_error());
    assert_eq!(response.stream_id(), 7);
    assert_eq!(response.data_as_size().unwrap(), 10_000);

    let response = read_file_contents(&request(FileContentsFlags::RANGE, 4096, 100), &entry);
    assert!(!response.is_error());
    assert_eq!(response.data(), &contents[4096..4196]);

    // A range running past the end returns what is left.
    let response = read_file_contents(&request(FileContentsFlags::RANGE, 9_990, 100), &entry);
    assert_eq!(response.data(), &contents[9_990..]);

    let response = read_file_contents(&request(FileContentsFlags::RANGE, 20_000, 100), &entry);
    assert!(!response.is_error());
    assert!(response.data().is_empty());
}

#[test]
fn file_contents_range_is_capped() {
    let temp = TempDir::new();
    let path = temp.path().join("big.bin");
    let chunk = usize::try_from(MAX_FILE_CONTENTS_CHUNK).unwrap();
    fs::write(&path, vec![0xA5u8; chunk + 10]).unwrap();
    let entry = file_entry(path);

    let response = read_file_contents(&request(FileContentsFlags::RANGE, 0, u32::MAX), &entry);
    assert_eq!(response.data().len(), chunk);
}

#[test]
fn file_contents_fail_for_directories_and_missing_files() {
    let temp = TempDir::new();

    let directory = LocalFileEntry {
        descriptor: FileDescriptor::new("dir").with_attributes(ClipboardFileAttributes::DIRECTORY),
        path: temp.path().to_path_buf(),
    };
    assert!(read_file_contents(&request(FileContentsFlags::SIZE, 0, 8), &directory).is_error());
    assert!(read_file_contents(&request(FileContentsFlags::RANGE, 0, 10), &directory).is_error());

    let missing = LocalFileEntry {
        descriptor: FileDescriptor::new("gone.txt"),
        path: temp.path().join("gone.txt"),
    };
    assert!(read_file_contents(&request(FileContentsFlags::SIZE, 0, 8), &missing).is_error());
    assert!(read_file_contents(&request(FileContentsFlags::RANGE, 0, 10), &missing).is_error());
}

#[test]
fn range_requests_stay_within_the_file_and_chunk_size() {
    // Unknown size: whatever is wanted, up to the chunk size.
    assert_eq!(range_request_len(0, None, 4096), 4096);
    assert_eq!(range_request_len(0, None, u32::MAX), MAX_FILE_CONTENTS_CHUNK);

    // Known size: never past the end, which `Cliprdr` would reject.
    assert_eq!(range_request_len(0, Some(100), 4096), 100);
    assert_eq!(range_request_len(90, Some(100), 4096), 10);
    assert_eq!(range_request_len(100, Some(100), 4096), 0);
    assert_eq!(range_request_len(200, Some(100), 4096), 0);

    // Huge files: positions past 4 GiB.
    let size = 6 * 1024 * 1024 * 1024u64;
    assert_eq!(range_request_len(size - 1, Some(size), 4096), 1);
    assert_eq!(range_request_len(1 << 32, Some(size), 4096), 4096);
}

#[test]
fn files_over_4_gib_need_huge_file_support() {
    let limit = u64::from(u32::MAX);
    assert!(fits_without_huge_file_support(&FileDescriptor::new("dir")));
    assert!(fits_without_huge_file_support(
        &FileDescriptor::new("a").with_file_size(limit)
    ));
    assert!(!fits_without_huge_file_support(
        &FileDescriptor::new("a").with_file_size(limit + 1)
    ));
}

#[test]
fn file_group_descriptor_round_trips() {
    let files = vec![
        FileDescriptor::new("dir").with_attributes(ClipboardFileAttributes::DIRECTORY),
        FileDescriptor::new("dir\\file.txt")
            .with_attributes(ClipboardFileAttributes::ARCHIVE)
            .with_last_write_time(0x01D9_0000_0000_0000)
            .with_file_size(5 * 1024 * 1024 * 1024),
    ];

    let data = encode_file_group_descriptor(&files).unwrap();
    // cItems followed by one 592-byte FILEDESCRIPTORW per file.
    assert_eq!(data.len(), 4 + 592 * 2);
    assert_eq!(&data[..4], &2u32.to_le_bytes());

    assert_eq!(decode_file_group_descriptor(&data).unwrap(), files);
    assert!(decode_file_group_descriptor(&data[..100]).is_none());
}
