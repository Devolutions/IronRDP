#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use ironrdp_core::ReadCursor;
use ironrdp_pdu as _;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCreateRequest, DeviceIoRequest,
    DeviceReadRequest, DeviceWriteRequest, FileAllocationInformation, FileAttributes, FileBasicInformation,
    FileDispositionInformation, FileEndOfFileInformation, FileInformationClass, FileInformationClassLevel,
    FileRenameInformation, FileStandardInformation, FileSystemInformationClassLevel, LockOperation, MajorFunction,
    MinorFunction, NtStatus, RdpLockInfo, ServerDriveIoRequest, ServerDriveLockControlRequest,
    ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest, ServerDriveQueryVolumeInformationRequest,
    ServerDriveSetInformationRequest, SharedAccess,
};
use ironrdp_rdpdr::{RdpdrBackend, RdpdrBackendFactory};
use ironrdp_rdpdr_native::backend::{NixRdpdrBackend, NixRdpdrBackendFactory, NixRdpdrBackendFactoryError};
use ironrdp_rdpdr_native::printer::PrintTarget;
use ironrdp_svc::SvcMessage;
use rstest::rstest;
use tracing as _;

// The read limit of the Windows drive backend.
const MAX_READ_LENGTH: u32 = 1_024 * 1_024;

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/path.rs
#[rstest]
#[case::parent_of_the_share(r"\..\escape.txt")]
#[case::through_a_subdirectory(r"\inside\..\..\escape.txt")]
#[case::forward_slash_separators(r"\inside/../../escape.txt")]
#[case::current_directory_component(r"\.\..\escape.txt")]
fn create_cannot_reach_outside_the_share(
    #[case] path: &str,
    #[values(CreateOptions::empty(), CreateOptions::FILE_DIRECTORY_FILE)] create_options: CreateOptions,
) {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("inside")).expect("create subdirectory");
    let mut backend = fixture.backend();

    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options,
            ..create_request(path, CreateDisposition::FILE_CREATE)
        },
    );

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/status.rs
    assert_eq!(response_status(&response), NtStatus::OBJECT_NAME_INVALID);
    assert!(!fixture.sandbox.join("escape.txt").exists());
}

// oracle: reference crates/ironrdp-rdpdr-native/tests/windows_backend.rs
#[rstest]
fn symbolic_links_in_the_share_are_not_followed(
    #[values(r"\dir_link\sentinel.txt", r"\file_link")] path: &str,
    #[values(CreateDisposition::FILE_OPEN, CreateDisposition::FILE_OVERWRITE_IF)] disposition: CreateDisposition,
) {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.sandbox.join("outside")).expect("create outside directory");
    let sentinel = fixture.sandbox.join("outside").join("sentinel.txt");
    std::fs::write(&sentinel, b"outside").expect("write sentinel");
    std::os::unix::fs::symlink(fixture.sandbox.join("outside"), fixture.share.join("dir_link"))
        .expect("link directory");
    std::os::unix::fs::symlink(&sentinel, fixture.share.join("file_link")).expect("link file");
    let mut backend = fixture.backend();

    let response = create_with(&mut backend, create_request(path, disposition));

    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(std::fs::read(&sentinel).expect("read sentinel"), b"outside");
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/path.rs
#[rstest]
#[case::asterisk(r"\a*b")]
#[case::question_mark(r"\a?b")]
// Windows takes `a:b` for a data stream, which Unix files lack, so the backend refuses it rather than create a file named so.
// oracle: mutant colon accepted, creating a file whose name holds the stream
#[case::colon(r"\a:b")]
#[case::less_than(r"\a<b")]
#[case::greater_than(r"\a>b")]
#[case::pipe(r"\a|b")]
#[case::quote("\\a\"b")]
#[case::control_character("\\a\u{1}b")]
#[case::trailing_dot(r"\name.")]
#[case::trailing_space(r"\name ")]
#[case::longer_than_255_units(&format!(r"\{}", "n".repeat(256)))]
fn names_windows_cannot_hold_are_refused(
    #[case] path: &str,
    #[values(CreateOptions::empty(), CreateOptions::FILE_DIRECTORY_FILE)] create_options: CreateOptions,
) {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();

    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options,
            ..create_request(path, CreateDisposition::FILE_CREATE)
        },
    );

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/status.rs
    assert_eq!(response_status(&response), NtStatus::OBJECT_NAME_INVALID);
    assert_eq!(std::fs::read_dir(&fixture.share).expect("list share").count(), 0);
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/path.rs
#[test]
fn rename_refuses_a_name_windows_cannot_hold() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\file.txt", CreateDisposition::FILE_CREATE);

    let response = set_information(&mut backend, file_id, rename_to(r"\a*b", false));

    assert_eq!(response_status(&response), NtStatus::OBJECT_NAME_INVALID);
    assert!(fixture.share.join("file.txt").exists());
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[rstest]
fn rename_refuses_a_target_without_a_name(
    #[values("", r"\")] file_name: &str,
    #[values(false, true)] replace_if_exists: bool,
) {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\file.txt", CreateDisposition::FILE_CREATE);

    let response = set_information(&mut backend, file_id, rename_to(file_name, replace_if_exists));

    assert_eq!(response_status(&response), NtStatus::OBJECT_NAME_INVALID);
    assert!(fixture.share.join("file.txt").exists());
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/directory.rs
#[rstest]
#[case::pipe(r"\a|b")]
#[case::colon(r"\a:b")]
#[case::empty_pattern(r"\dir\")]
fn query_directory_refuses_an_invalid_pattern(#[case] path: &str) {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    let mut backend = fixture.backend();
    let root_id = create(&mut backend, r"\", CreateDisposition::FILE_OPEN);

    let response = query_directory(&mut backend, root_id, path, true);

    assert_eq!(response_status(&response), NtStatus::OBJECT_NAME_INVALID);
}

/// The DOS wildcards are valid in a pattern, so a pattern without a match reports no match rather than an invalid name.
// oracle: spec MS-FSA §2.1.5.6.3
#[rstest]
#[case::dos_star(r"\<.none")]
#[case::dos_question_mark(r"\>.none")]
#[case::dos_dot("\\a\"none")]
fn a_dos_wildcard_pattern_without_a_match_finds_no_file(#[case] path: &str) {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("a.txt"), b"a").expect("write file");
    let mut backend = fixture.backend();
    let root_id = create(&mut backend, r"\", CreateDisposition::FILE_OPEN);

    let response = query_directory(&mut backend, root_id, path, true);

    assert_eq!(response_status(&response), NtStatus::NO_SUCH_FILE);
}

/// An initial query that cannot open its directory reports why, rather than an empty match.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/directory.rs
#[test]
fn an_initial_query_through_a_file_reports_it_is_not_a_directory() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("file.txt"), b"file").expect("write file");
    let mut backend = fixture.backend();
    let root_id = create(&mut backend, r"\", CreateDisposition::FILE_OPEN);

    let response = query_directory(&mut backend, root_id, r"\file.txt\*", true);

    assert_eq!(response_status(&response), NtStatus::NOT_A_DIRECTORY);
}

/// Opening a FIFO without a writer must not block, because a blocked open stalls the whole channel.
// oracle: mutant open without O_NONBLOCK
#[test]
fn opening_a_fifo_in_the_share_does_not_block() {
    let fixture = Fixture::new();
    nix::unistd::mkfifo(
        &fixture.share.join("fifo"),
        nix::sys::stat::Mode::from_bits_truncate(0o600),
    )
    .expect("create FIFO");
    let mut backend = fixture.backend();

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let response = backend.handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
            r"\fifo",
            CreateDisposition::FILE_OPEN,
        )));
        let _ = sender.send(response.is_ok());
    });

    let completed = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the create request completes without waiting for a FIFO writer");
    assert!(completed);
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/path.rs
#[test]
fn rename_cannot_move_a_file_outside_the_share() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\inside.txt", CreateDisposition::FILE_CREATE);

    let response = set_information(&mut backend, file_id, rename_to(r"\..\moved.txt", false));

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/status.rs
    assert_eq!(response_status(&response), NtStatus::OBJECT_NAME_INVALID);
    assert!(fixture.share.join("inside.txt").exists());
    assert!(!fixture.sandbox.join("moved.txt").exists());
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/directory.rs
#[test]
fn query_directory_cannot_enumerate_outside_the_share() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\", CreateDisposition::FILE_OPEN);

    let response = query_directory(&mut backend, file_id, r"\..\*", true);

    assert_eq!(response_status(&response), NtStatus::OBJECT_NAME_INVALID);
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[rstest]
#[case::at_the_limit(MAX_READ_LENGTH, NtStatus::SUCCESS)]
#[case::one_past_the_limit(MAX_READ_LENGTH + 1, NtStatus::INVALID_PARAMETER)]
#[case::u32_max(u32::MAX, NtStatus::INVALID_PARAMETER)]
fn reads_longer_than_the_limit_are_rejected(#[case] length: u32, #[case] status: NtStatus) {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("data.bin"), vec![7; 16]).expect("write data");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\data.bin", CreateDisposition::FILE_OPEN);

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
            device_io_request: request_header(file_id, MajorFunction::Read),
            length,
            offset: 0,
        }))
        .expect("complete read");

    assert_eq!(response_status(&response), status);
}

// oracle: spec MS-FSA §2.1.5.1
#[rstest]
// oracle: spec MS-FSA §2.1.5.1.2
#[case::existing_file_with_create(
    r"\file.txt",
    CreateOptions::empty(),
    CreateDisposition::FILE_CREATE,
    NtStatus::OBJECT_NAME_COLLISION
)]
// oracle: spec MS-FSA §2.1.5.1.2
#[case::existing_directory_with_create(
    r"\dir",
    CreateOptions::empty(),
    CreateDisposition::FILE_CREATE,
    NtStatus::OBJECT_NAME_COLLISION
)]
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/status.rs
#[case::missing_name(
    r"\missing.txt",
    CreateOptions::empty(),
    CreateDisposition::FILE_OPEN,
    NtStatus::NO_SUCH_FILE
)]
#[case::missing_parent(
    r"\missing\file.txt",
    CreateOptions::empty(),
    CreateDisposition::FILE_OPEN_IF,
    NtStatus::OBJECT_PATH_NOT_FOUND
)]
#[case::directory_as_file(
    r"\dir",
    CreateOptions::FILE_NON_DIRECTORY_FILE,
    CreateDisposition::FILE_OPEN,
    NtStatus::FILE_IS_A_DIRECTORY
)]
#[case::file_as_directory(
    r"\file.txt",
    CreateOptions::FILE_DIRECTORY_FILE,
    CreateDisposition::FILE_OPEN,
    NtStatus::NOT_A_DIRECTORY
)]
// oracle: spec MS-SMB2 §2.2.13
#[case::directory_and_non_directory(
    r"\new",
    CreateOptions::FILE_DIRECTORY_FILE.union(CreateOptions::FILE_NON_DIRECTORY_FILE),
    CreateDisposition::FILE_CREATE,
    NtStatus::INVALID_PARAMETER
)]
// oracle: spec MS-SMB2 §2.2.13
#[case::directory_with_overwrite(
    r"\new",
    CreateOptions::FILE_DIRECTORY_FILE,
    CreateDisposition::FILE_OVERWRITE_IF,
    NtStatus::INVALID_PARAMETER
)]
// oracle: spec MS-SMB2 §2.2.13
#[case::invalid_disposition_for_a_missing_name(
    r"\missing.txt",
    CreateOptions::empty(),
    CreateDisposition::from(6),
    NtStatus::INVALID_PARAMETER
)]
// oracle: spec MS-SMB2 §2.2.13
#[case::invalid_disposition_for_a_directory(
    r"\dir",
    CreateOptions::empty(),
    CreateDisposition::from(6),
    NtStatus::INVALID_PARAMETER
)]
fn create_reports_the_failure_status(
    #[case] path: &str,
    #[case] create_options: CreateOptions,
    #[case] disposition: CreateDisposition,
    #[case] status: NtStatus,
) {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("file.txt"), b"file").expect("write file");
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    let mut backend = fixture.backend();

    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options,
            ..create_request(path, disposition)
        },
    );

    assert_eq!(response_status(&response), status);
}

/// Each operation that resolves a server-supplied path works on paths inside a subdirectory of the share.
#[test]
fn file_lifecycle_inside_a_subdirectory() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("docs")).expect("create subdirectory");
    let mut backend = fixture.backend();

    // oracle: spec MS-FSA §2.1.5.1.1
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options: CreateOptions::FILE_DIRECTORY_FILE,
            ..create_request(r"\docs\new", CreateDisposition::FILE_CREATE)
        },
    );
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(fixture.share.join("docs").join("new").is_dir());
    close(&mut backend, response_file_id(&response));

    let file_id = create(&mut backend, r"\docs\report.txt", CreateDisposition::FILE_CREATE);
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"hello".to_vec(),
        }))
        .expect("complete write");
    // oracle: spec MS-FSA §2.1.5.4
    assert_eq!(response_status(&response), NtStatus::SUCCESS);

    // oracle: spec MS-FSA §2.1.5.15.12
    let response = set_information(&mut backend, file_id, rename_to(r"\docs\new\renamed.txt", false));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let renamed = fixture.share.join("docs").join("new").join("renamed.txt");
    assert_eq!(std::fs::read(&renamed).expect("read renamed file"), b"hello");

    // The handle follows the rename, so the delete applies to the renamed file when the handle closes.
    // oracle: spec MS-FSCC §2.4.11
    let response = set_information(&mut backend, file_id, delete_pending(true));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    close(&mut backend, file_id);
    assert!(!renamed.exists());
}

// MS-FSA §2.1.5.6.3 omits `.` and `..` for a pattern that matches ".", as `*` does, but Windows lists them.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/directory.rs
#[test]
fn a_subdirectory_lists_dot_entries_first() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("docs")).expect("create subdirectory");
    std::fs::write(fixture.share.join("docs").join("a.txt"), b"a").expect("write file");
    let mut backend = fixture.backend();
    let docs_id = create(&mut backend, r"\docs", CreateDisposition::FILE_OPEN);

    assert_eq!(
        list_directory(&mut backend, docs_id, r"\docs\*"),
        (
            vec![".".to_owned(), "..".to_owned(), "a.txt".to_owned()],
            NtStatus::NO_MORE_FILES
        )
    );
}

// oracle: spec MS-FSA §2.1.5.6.3
#[test]
fn the_shared_directory_lists_no_dot_entries() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("only.txt"), b"only").expect("write file");
    let mut backend = fixture.backend();
    let root_id = create(&mut backend, r"\", CreateDisposition::FILE_OPEN);

    assert_eq!(
        list_directory(&mut backend, root_id, r"\*"),
        (vec!["only.txt".to_owned()], NtStatus::NO_MORE_FILES)
    );
}

// oracle: mutant a rejected initial query lets a continuation resume the earlier listing
// No specification decides this: MS-FSA §2.1.5.6.3 fails an invalid pattern before it resets the enumeration position,
// and the rejected path, which leaves the share, is a client policy rather than a pattern MS-FSA describes.
#[test]
fn a_failed_initial_query_restarts_enumeration() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("a.txt"), b"a").expect("write file");
    std::fs::write(fixture.share.join("b.txt"), b"b").expect("write file");
    let mut backend = fixture.backend();
    let root_id = create(&mut backend, r"\", CreateDisposition::FILE_OPEN);

    let first = query_directory(&mut backend, root_id, r"\*", true);
    let invalid = query_directory(&mut backend, root_id, r"\..\*", true);
    let next = query_directory(&mut backend, root_id, "", false);

    assert_eq!(response_status(&first), NtStatus::SUCCESS);
    assert_eq!(response_status(&invalid), NtStatus::OBJECT_NAME_INVALID);
    assert_eq!(response_status(&next), NtStatus::NO_MORE_FILES);
}

// oracle: spec MS-SMB2 §2.2.14
#[test]
fn supersede_replaces_the_contents() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("old.txt"), b"old contents").expect("write file");
    let mut backend = fixture.backend();

    let _file_id = create(&mut backend, r"\old.txt", CreateDisposition::FILE_SUPERSEDE);

    assert_eq!(std::fs::read(fixture.share.join("old.txt")).expect("read file"), b"");
}

// oracle: spec MS-FSCC §2.4.42
#[test]
fn rename_without_replace_keeps_an_existing_target() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("target.txt"), b"keep").expect("write target");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\source.txt", CreateDisposition::FILE_CREATE);

    let response = set_information(&mut backend, file_id, rename_to(r"\target.txt", false));

    assert_eq!(response_status(&response), NtStatus::OBJECT_NAME_COLLISION);
    assert_eq!(
        std::fs::read(fixture.share.join("target.txt")).expect("read target"),
        b"keep"
    );
    assert!(fixture.share.join("source.txt").exists());
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn rename_with_replace_overwrites_an_existing_file() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("target.txt"), b"old").expect("write target");
    std::fs::write(fixture.share.join("source.txt"), b"new").expect("write source");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\source.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, file_id, rename_to(r"\target.txt", true));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(
        std::fs::read(fixture.share.join("target.txt")).expect("read target"),
        b"new"
    );
    assert!(!fixture.share.join("source.txt").exists());
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn rename_with_replace_refuses_a_directory_target() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("target")).expect("create target directory");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\source", CreateDisposition::FILE_CREATE);

    let response = set_information(&mut backend, file_id, rename_to(r"\target", true));

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert!(fixture.share.join("target").is_dir());
    assert!(fixture.share.join("source").exists());
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn rename_onto_the_same_file_is_not_a_collision() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("same.txt"), b"same").expect("write file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\same.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, file_id, rename_to(r"\same.txt", false));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(fixture.share.join("same.txt").exists());
}

// oracle: mutant missing rename target directory reported as a missing file
// No source decides this status: MS-FSA §2.1.5.15.12 read literally, and the Windows backend, report STATUS_OBJECT_NAME_NOT_FOUND.
// STATUS_OBJECT_PATH_NOT_FOUND is kept to match a create below a missing directory.
#[test]
fn rename_into_a_missing_directory_reports_the_path() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\file.txt", CreateDisposition::FILE_CREATE);

    let response = set_information(&mut backend, file_id, rename_to(r"\missing\file.txt", false));

    assert_eq!(response_status(&response), NtStatus::OBJECT_PATH_NOT_FOUND);
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn a_file_marked_for_deletion_cannot_be_renamed() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doomed.txt"), b"doomed").expect("write file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\doomed.txt", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, file_id, delete_pending(true));

    let response = set_information(&mut backend, file_id, rename_to(r"\saved.txt", false));

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
}

// MS-FSA §2.1.5.15.12 moves only the renaming handle to the new link, but Windows deletes the renamed file.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[test]
fn delete_on_close_follows_a_rename_through_another_handle() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doc.txt"), b"original").expect("write file");
    let mut backend = fixture.backend();
    let deleting_id = create(&mut backend, r"\doc.txt", CreateDisposition::FILE_OPEN);
    let renaming_id = create(&mut backend, r"\doc.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, renaming_id, rename_to(r"\backup.tmp", false));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    std::fs::write(fixture.share.join("doc.txt"), b"replacement").expect("write replacement");
    let _ = set_information(&mut backend, deleting_id, delete_pending(true));
    close(&mut backend, deleting_id);
    close(&mut backend, renaming_id);

    assert_eq!(
        std::fs::read(fixture.share.join("doc.txt")).expect("read replacement"),
        b"replacement"
    );
    assert!(!fixture.share.join("backup.tmp").exists());
}

/// Renaming one hard link leaves a handle opened through another link on that other link.
// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn renaming_a_hard_link_does_not_move_handles_opened_through_another_link() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("a.txt"), b"linked").expect("write file");
    std::fs::hard_link(fixture.share.join("a.txt"), fixture.share.join("b.txt")).expect("link file");
    let mut backend = fixture.backend();
    let a_id = create(&mut backend, r"\a.txt", CreateDisposition::FILE_OPEN);
    let b_id = create(&mut backend, r"\b.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, a_id, rename_to(r"\c.txt", false));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let _ = set_information(&mut backend, b_id, delete_pending(true));
    close(&mut backend, a_id);
    close(&mut backend, b_id);

    assert!(!fixture.share.join("b.txt").exists());
    assert_eq!(
        std::fs::read(fixture.share.join("c.txt")).expect("read c.txt"),
        b"linked"
    );
}

/// A backend with only a shared directory opens it by path for each request, so a handle is judged by what its path names in the directory now at that path.
/// A handle opened through another link before a local process replaced the shared directory still names its file there, and keeps that link.
// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn renaming_a_hard_link_does_not_move_a_handle_of_another_link_opened_before_the_shared_directory_was_replaced() {
    let fixture = Fixture::new();
    let moved = fixture.sandbox.join("moved");
    std::fs::write(fixture.share.join("p1.txt"), b"linked").expect("write file");
    std::fs::hard_link(fixture.share.join("p1.txt"), fixture.share.join("p2.txt")).expect("link file");
    let mut backend = fixture.backend();
    let p2_id = create(&mut backend, r"\p2.txt", CreateDisposition::FILE_OPEN);
    std::fs::rename(&fixture.share, &moved).expect("move shared directory");
    std::fs::create_dir(&fixture.share).expect("create directory");
    std::fs::hard_link(moved.join("p1.txt"), fixture.share.join("p1.txt")).expect("link file");
    std::fs::hard_link(moved.join("p1.txt"), fixture.share.join("p2.txt")).expect("link file");
    let p1_id = create(&mut backend, r"\p1.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, p1_id, rename_to(r"\t.txt", false));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let _ = set_information(&mut backend, p2_id, delete_pending(true));
    close(&mut backend, p1_id);
    close(&mut backend, p2_id);

    assert_eq!(names_in(&fixture.share), ["t.txt"]);
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn rename_with_replace_refuses_a_target_marked_for_deletion() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("target.txt"), b"old").expect("write target");
    std::fs::write(fixture.share.join("source.txt"), b"new").expect("write source");
    let mut backend = fixture.backend();
    let target_id = create(&mut backend, r"\target.txt", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, target_id, delete_pending(true));
    let source_id = create(&mut backend, r"\source.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, source_id, rename_to(r"\target.txt", true));

    assert_eq!(response_status(&response), NtStatus::DELETE_PENDING);
    assert_eq!(
        std::fs::read(fixture.share.join("target.txt")).expect("read target"),
        b"old"
    );
    assert!(fixture.share.join("source.txt").exists());
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn rename_with_replace_refuses_a_target_with_open_handles() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("target.txt"), b"old").expect("write target");
    std::fs::write(fixture.share.join("source.txt"), b"new").expect("write source");
    let mut backend = fixture.backend();
    let _target_id = create(&mut backend, r"\target.txt", CreateDisposition::FILE_OPEN);
    let source_id = create(&mut backend, r"\source.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, source_id, rename_to(r"\target.txt", true));

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert_eq!(
        std::fs::read(fixture.share.join("target.txt")).expect("read target"),
        b"old"
    );
    assert!(fixture.share.join("source.txt").exists());
}

/// Renaming onto another hard link of the same file removes the source link, where POSIX rename changes nothing.
// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn rename_onto_another_link_of_the_same_file_removes_the_source() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("a.txt"), b"linked").expect("write file");
    std::fs::hard_link(fixture.share.join("a.txt"), fixture.share.join("b.txt")).expect("link file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\a.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, file_id, rename_to(r"\b.txt", false));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(!fixture.share.join("a.txt").exists());
    assert_eq!(
        std::fs::read(fixture.share.join("b.txt")).expect("read b.txt"),
        b"linked"
    );
}

/// A rename moves only the file the handle holds, never a file that took its name since it was opened.
/// On a case-sensitive filesystem, a link whose name differs from the source only in case is another link, so the source goes.
// oracle: spec MS-FSA §2.1.5.15.12
#[cfg(target_os = "linux")]
#[test]
fn rename_onto_a_link_differing_only_in_case_removes_the_source() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("a.txt"), b"linked").expect("write file");
    std::fs::hard_link(fixture.share.join("a.txt"), fixture.share.join("A.txt"))
        .expect("the share must be on a case-sensitive filesystem");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\a.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, file_id, rename_to(r"\A.txt", false));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(!fixture.share.join("a.txt").exists());
    assert_eq!(
        std::fs::read(fixture.share.join("A.txt")).expect("read A.txt"),
        b"linked"
    );
}

/// On a filesystem that ignores Unicode normalization, another normalization of the source name is the source itself, so the file stays.
// oracle: spec MS-FSA §2.1.5.15.12
#[cfg(target_os = "macos")]
#[test]
fn rename_to_another_normalization_of_the_name_keeps_the_file() {
    let decomposed = "e\u{301}.txt";
    let composed = "\u{e9}.txt";
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join(decomposed), b"accented").expect("write file");
    assert!(
        fixture.share.join(composed).exists(),
        "the share must be on a normalization-insensitive filesystem"
    );
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, &format!(r"\{decomposed}"), CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, file_id, rename_to(&format!(r"\{composed}"), false));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(
        std::fs::read(fixture.share.join(composed)).expect("read file"),
        b"accented"
    );
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn rename_never_moves_a_file_that_replaced_the_handle_name() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doc.txt"), b"original").expect("write file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\doc.txt", CreateDisposition::FILE_OPEN);
    std::fs::rename(fixture.share.join("doc.txt"), fixture.share.join("moved.txt")).expect("rename locally");
    std::fs::write(fixture.share.join("doc.txt"), b"replacement").expect("write replacement");

    let response = set_information(&mut backend, file_id, rename_to(r"\renamed.txt", false));

    assert_eq!(
        std::fs::read(fixture.share.join("doc.txt")).expect("read replacement"),
        b"replacement"
    );
    // MS-FSA would rename the handle's file, which no longer has a name the backend can reach.
    // oracle: mutant rename reports success without moving the handle's file
    assert_ne!(response_status(&response), NtStatus::SUCCESS);
}

// oracle: spec MS-FSA §2.1.5.12.27
#[test]
fn a_link_marked_for_deletion_is_not_counted() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("a.txt"), b"linked").expect("write file");
    std::fs::hard_link(fixture.share.join("a.txt"), fixture.share.join("b.txt")).expect("link file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\a.txt", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, file_id, delete_pending(true));

    let information = query_standard_information(&mut backend, file_id);

    assert_eq!(information.number_of_links, 1);
    assert_eq!(information.delete_pending, Boolean::True);
}

/// A directory has one link, however many entries Unix counts for `.` and subdirectories.
// oracle: spec MS-FSA §2.1.5.12.27
#[rstest]
#[case::unmarked(false, 1)]
#[case::marked_for_deletion(true, 0)]
fn a_directory_counts_one_link(#[case] marked: bool, #[case] links: u32) {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, directory_id, delete_pending(marked));

    assert_eq!(
        query_standard_information(&mut backend, directory_id).number_of_links,
        links
    );
}

/// The shared directory cannot be removed through the share, so it is never marked, and creates in it keep working.
// oracle: mutant marking the shared directory blocks every create in it
#[test]
fn the_shared_directory_cannot_be_marked_for_deletion() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    let root_id = create(&mut backend, r"\", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, root_id, delete_pending(true));
    let created = create_with(
        &mut backend,
        create_request(r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert_eq!(response_status(&created), NtStatus::SUCCESS);
}

/// Dropping the backend closes every handle the server left open, so their pending deletes run.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/handles.rs
#[test]
fn dropping_the_backend_runs_pending_deletes() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doc.txt"), b"doc").expect("write file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\doc.txt", CreateDisposition::FILE_OPEN);
    let response = set_information(&mut backend, file_id, delete_pending(true));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);

    drop(backend);

    assert!(!fixture.share.join("doc.txt").exists());
}

/// A delete on close never removes a file that took the marked name after the handle was opened.
// oracle: spec MS-FSA §2.1.5.5
#[test]
fn a_delete_on_close_keeps_a_file_that_replaced_the_marked_name() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doc.txt"), b"original").expect("write file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\doc.txt", CreateDisposition::FILE_OPEN);
    std::fs::rename(fixture.share.join("doc.txt"), fixture.share.join("moved.txt")).expect("rename locally");
    std::fs::write(fixture.share.join("doc.txt"), b"replacement").expect("write replacement");

    let _ = set_information(&mut backend, file_id, delete_pending(true));
    close(&mut backend, file_id);

    assert_eq!(
        std::fs::read(fixture.share.join("doc.txt")).expect("read replacement"),
        b"replacement"
    );
}

/// The delete removes the hard link it was marked through, whichever handle to the file closes last.
// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn a_delete_removes_the_hard_link_it_was_marked_through() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("a.txt"), b"linked").expect("write file");
    std::fs::hard_link(fixture.share.join("a.txt"), fixture.share.join("b.txt")).expect("link file");
    let mut backend = fixture.backend();
    let a_id = create(&mut backend, r"\a.txt", CreateDisposition::FILE_OPEN);
    let b_id = create(&mut backend, r"\b.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, b_id, delete_pending(true));
    close(&mut backend, b_id);
    close(&mut backend, a_id);

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(!fixture.share.join("b.txt").exists());
    assert_eq!(
        std::fs::read(fixture.share.join("a.txt")).expect("read a.txt"),
        b"linked"
    );
}

/// On a case-insensitive filesystem, a handle opened under another spelling of the renamed name follows the rename.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[cfg(target_os = "macos")]
#[test]
fn a_handle_opened_under_another_spelling_follows_a_rename() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doc.txt"), b"doc").expect("write file");
    assert!(
        fixture.share.join("DOC.TXT").exists(),
        "the share must be on a case-insensitive filesystem"
    );
    let mut backend = fixture.backend();
    let deleting_id = create(&mut backend, r"\Doc.txt", CreateDisposition::FILE_OPEN);
    let renaming_id = create(&mut backend, r"\doc.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, renaming_id, rename_to(r"\renamed.txt", false));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let _ = set_information(&mut backend, deleting_id, delete_pending(true));
    close(&mut backend, renaming_id);
    close(&mut backend, deleting_id);

    assert!(!fixture.share.join("renamed.txt").exists());
}

// oracle: spec MS-FSA §2.1.5.5
#[test]
fn a_file_marked_for_deletion_is_deleted_when_its_last_handle_closes() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("shared.txt"), b"shared").expect("write file");
    let mut backend = fixture.backend();
    let first_id = create(&mut backend, r"\shared.txt", CreateDisposition::FILE_OPEN);
    let second_id = create(&mut backend, r"\shared.txt", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, first_id, delete_pending(true));

    let reopen = create_with(
        &mut backend,
        create_request(r"\shared.txt", CreateDisposition::FILE_OPEN),
    );
    close(&mut backend, first_id);
    let still_there = fixture.share.join("shared.txt").exists();
    close(&mut backend, second_id);

    // MS-FSA §2.1.5.1 refuses only the directories on the path, but Windows also refuses the marked file itself.
    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
    assert_eq!(response_status(&reopen), NtStatus::DELETE_PENDING);
    assert!(still_there);
    assert!(!fixture.share.join("shared.txt").exists());
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[rstest]
fn overwriting_a_file_marked_for_deletion_keeps_its_contents(
    #[values(
        CreateDisposition::FILE_SUPERSEDE,
        CreateDisposition::FILE_OVERWRITE,
        CreateDisposition::FILE_OVERWRITE_IF
    )]
    disposition: CreateDisposition,
) {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("held.txt"), b"held").expect("write file");
    let mut backend = fixture.backend();
    let marking_id = create(&mut backend, r"\held.txt", CreateDisposition::FILE_OPEN);
    let _holding_id = create(&mut backend, r"\held.txt", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, marking_id, delete_pending(true));

    let response = create_with(&mut backend, create_request(r"\held.txt", disposition));

    assert_eq!(response_status(&response), NtStatus::DELETE_PENDING);
    assert_eq!(
        std::fs::read(fixture.share.join("held.txt")).expect("read file"),
        b"held"
    );
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn a_directory_holding_open_files_cannot_be_renamed() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    std::fs::write(fixture.share.join("dir").join("open.txt"), b"open").expect("write file");
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);
    let _file_id = create(&mut backend, r"\dir\open.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, directory_id, rename_to(r"\moved", false));

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert!(fixture.share.join("dir").join("open.txt").exists());
}

/// The open files are found by the directory that holds them, not by the paths they were opened by.
// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn a_directory_holding_open_files_cannot_be_renamed_after_a_local_rename() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    std::fs::write(fixture.share.join("dir").join("open.txt"), b"open").expect("write file");
    let mut backend = fixture.backend();
    let _file_id = create(&mut backend, r"\dir\open.txt", CreateDisposition::FILE_OPEN);
    std::fs::rename(fixture.share.join("dir"), fixture.share.join("renamed")).expect("rename locally");
    let directory_id = create(&mut backend, r"\renamed", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, directory_id, rename_to(r"\moved", false));

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert!(fixture.share.join("renamed").join("open.txt").exists());
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn a_directory_holding_a_file_renamed_into_it_cannot_be_renamed() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("a")).expect("create directory");
    std::fs::create_dir(fixture.share.join("b")).expect("create directory");
    std::fs::write(fixture.share.join("a").join("open.txt"), b"open").expect("write file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\a\open.txt", CreateDisposition::FILE_OPEN);
    let response = set_information(&mut backend, file_id, rename_to(r"\b\open.txt", false));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let directory_id = create(&mut backend, r"\b", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, directory_id, rename_to(r"\moved", false));

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert!(fixture.share.join("b").join("open.txt").exists());
}

/// On a case-insensitive filesystem, a file opened under another spelling of its directory still keeps that directory from being renamed.
// oracle: spec MS-FSA §2.1.5.15.12
#[cfg(target_os = "macos")]
#[test]
fn a_directory_holding_files_open_under_another_spelling_cannot_be_renamed() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    std::fs::write(fixture.share.join("dir").join("open.txt"), b"open").expect("write file");
    assert!(
        fixture.share.join("DIR").exists(),
        "the share must be on a case-insensitive filesystem"
    );
    let mut backend = fixture.backend();
    let _file_id = create(&mut backend, r"\DIR\open.txt", CreateDisposition::FILE_OPEN);
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, directory_id, rename_to(r"\moved", false));

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
}

// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn an_open_directory_without_open_files_can_be_renamed() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    std::fs::write(fixture.share.join("dir").join("closed.txt"), b"closed").expect("write file");
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, directory_id, rename_to(r"\moved", false));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(fixture.share.join("moved").join("closed.txt").exists());
}

// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn clearing_delete_pending_before_close_keeps_the_file() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("kept.txt"), b"kept").expect("write file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\kept.txt", CreateDisposition::FILE_OPEN);

    let set = set_information(&mut backend, file_id, delete_pending(true));
    let clear = set_information(&mut backend, file_id, delete_pending(false));
    close(&mut backend, file_id);

    assert_eq!(response_status(&set), NtStatus::SUCCESS);
    assert_eq!(response_status(&clear), NtStatus::SUCCESS);
    assert!(fixture.share.join("kept.txt").exists());
}

// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn an_empty_directory_opened_with_open_if_is_deleted_on_close() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("empty")).expect("create directory");
    let mut backend = fixture.backend();
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options: CreateOptions::FILE_DIRECTORY_FILE,
            ..create_request(r"\empty", CreateDisposition::FILE_OPEN_IF)
        },
    );
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let directory_id = response_file_id(&response);

    let response = set_information(&mut backend, directory_id, delete_pending(true));
    close(&mut backend, directory_id);

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(!fixture.share.join("empty").exists());
}

// oracle: spec MS-FSA §2.1.5.1
#[rstest]
fn nothing_can_be_created_in_a_directory_marked_for_deletion(
    #[values(CreateOptions::empty(), CreateOptions::FILE_DIRECTORY_FILE)] create_options: CreateOptions,
) {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, directory_id, delete_pending(true));

    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options,
            ..create_request(r"\dir\x", CreateDisposition::FILE_CREATE)
        },
    );
    let created = fixture.share.join("dir").join("x").exists();
    close(&mut backend, directory_id);

    assert_eq!(response_status(&response), NtStatus::DELETE_PENDING);
    assert!(!created);
    assert!(!fixture.share.join("dir").exists());
}

/// Every directory on the path counts, so a subdirectory a local process created in a marked directory gains nothing either.
// oracle: spec MS-FSA §2.1.5.1
#[test]
fn nothing_can_be_created_below_a_directory_marked_for_deletion() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, directory_id, delete_pending(true));
    std::fs::create_dir(fixture.share.join("dir").join("sub")).expect("create subdirectory locally");

    let response = create_with(
        &mut backend,
        create_request(r"\dir\sub\x.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(response_status(&response), NtStatus::DELETE_PENDING);
    assert!(!fixture.share.join("dir").join("sub").join("x.txt").exists());
}

/// A rename opens its target directory like a create does, so it cannot move a file into a directory marked for deletion.
// The target directory is the last component of that open, which MS-FSA §2.1.5.1 does not refuse, but Windows does.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[test]
fn nothing_can_be_renamed_into_a_directory_marked_for_deletion() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    std::fs::write(fixture.share.join("file.txt"), b"file").expect("write file");
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, directory_id, delete_pending(true));
    let file_id = create(&mut backend, r"\file.txt", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, file_id, rename_to(r"\dir\file.txt", false));
    close(&mut backend, directory_id);

    assert_eq!(response_status(&response), NtStatus::DELETE_PENDING);
    assert!(fixture.share.join("file.txt").exists());
    assert!(!fixture.share.join("dir").exists());
}

/// The emptiness check applies to the open directory, not to whatever now holds the name it was opened by.
// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn marking_a_directory_checks_the_open_directory_after_a_local_rename() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);
    std::fs::rename(fixture.share.join("dir"), fixture.share.join("moved")).expect("rename directory");
    std::fs::create_dir(fixture.share.join("dir")).expect("create replacement");
    std::fs::write(fixture.share.join("dir").join("child.txt"), b"child").expect("write child");

    let response = set_information(&mut backend, directory_id, delete_pending(true));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
}

/// Each mark reads the directory from its start, so an earlier mark does not hide entries added since.
// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn marking_a_directory_again_sees_entries_added_since() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);
    let first = set_information(&mut backend, directory_id, delete_pending(true));
    let _ = set_information(&mut backend, directory_id, delete_pending(false));
    std::fs::write(fixture.share.join("dir").join("child.txt"), b"child").expect("write child");

    let second = set_information(&mut backend, directory_id, delete_pending(true));

    assert_eq!(response_status(&first), NtStatus::SUCCESS);
    assert_eq!(response_status(&second), NtStatus::DIRECTORY_NOT_EMPTY);
}

// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn a_directory_that_is_not_empty_cannot_be_marked_for_deletion() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("full")).expect("create directory");
    std::fs::write(fixture.share.join("full").join("child.txt"), b"child").expect("write child");
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\full", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, directory_id, delete_pending(true));
    close(&mut backend, directory_id);

    assert_eq!(response_status(&response), NtStatus::DIRECTORY_NOT_EMPTY);
    assert!(fixture.share.join("full").join("child.txt").exists());
}

// oracle: spec MS-SMB2 §2.2.13.1
#[test]
fn opening_an_existing_file_for_write_allows_writes() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doc.txt"), b"old").expect("write file");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\doc.txt", CreateDisposition::FILE_OPEN);

    let response = write(&mut backend, file_id, 0, b"new");

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(std::fs::read(fixture.share.join("doc.txt")).expect("read file"), b"new");
}

// oracle: spec MS-SMB2 §3.3.5.13
#[test]
fn a_handle_opened_for_reading_cannot_write() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doc.txt"), b"old").expect("write file");
    let mut backend = fixture.backend();
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            desired_access: DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY,
            ..create_request(r"\doc.txt", CreateDisposition::FILE_OPEN)
        },
    );
    let file_id = response_file_id(&response);

    let response = write(&mut backend, file_id, 0, b"new");

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert_eq!(std::fs::read(fixture.share.join("doc.txt")).expect("read file"), b"old");
}

// oracle: spec MS-FSA §2.1.5.5
#[test]
fn delete_on_close_removes_the_file_when_its_handle_closes() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            desired_access: DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE | DesiredAccess::DELETE,
            create_options: CreateOptions::FILE_DELETE_ON_CLOSE,
            ..create_request(r"\temp.tmp", CreateDisposition::FILE_CREATE)
        },
    );
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let reopened = create(&mut backend, r"\temp.tmp", CreateDisposition::FILE_OPEN);
    close(&mut backend, reopened);
    let exists_while_open = fixture.share.join("temp.tmp").exists();

    close(&mut backend, response_file_id(&response));

    assert!(exists_while_open);
    assert!(!fixture.share.join("temp.tmp").exists());
}

// oracle: spec MS-FSA §2.1.5.1
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[rstest]
#[case::write_only(DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE)]
#[case::maximum_allowed(DesiredAccess::MAXIMUM_ALLOWED)]
#[case::generic_all(DesiredAccess::GENERIC_ALL)]
fn delete_on_close_needs_the_delete_bit(#[case] desired_access: DesiredAccess) {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();

    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            desired_access,
            create_options: CreateOptions::FILE_DELETE_ON_CLOSE,
            ..create_request(r"\temp.tmp", CreateDisposition::FILE_CREATE)
        },
    );

    assert_eq!(response_status(&response), NtStatus::INVALID_PARAMETER);
    assert!(!fixture.share.join("temp.tmp").exists());
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[test]
fn the_share_cannot_be_deleted_on_close() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();

    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options: CreateOptions::FILE_DELETE_ON_CLOSE,
            ..create_request(r"\", CreateDisposition::FILE_OPEN)
        },
    );

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert!(fixture.share.exists());
}

/// Another handle keeps the directory open across the close, so a mark would show as a refused create below it.
// oracle: spec MS-FSA §2.1.5.5
#[test]
fn delete_on_close_keeps_a_directory_that_is_not_empty() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    std::fs::write(fixture.share.join("dir").join("kept.txt"), b"kept").expect("write file");
    let mut backend = fixture.backend();
    let other_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options: CreateOptions::FILE_DELETE_ON_CLOSE | CreateOptions::FILE_DIRECTORY_FILE,
            ..create_request(r"\dir", CreateDisposition::FILE_OPEN)
        },
    );
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    close(&mut backend, response_file_id(&response));

    let created = create_with(
        &mut backend,
        create_request(r"\dir\new.txt", CreateDisposition::FILE_CREATE),
    );
    close(&mut backend, other_id);

    assert_eq!(response_status(&created), NtStatus::SUCCESS);
    assert!(fixture.share.join("dir").join("kept.txt").exists());
}

// oracle: spec MS-FSA §2.1.5.5
#[test]
fn delete_on_close_removes_an_empty_directory() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("empty")).expect("create directory");
    let mut backend = fixture.backend();
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options: CreateOptions::FILE_DELETE_ON_CLOSE | CreateOptions::FILE_DIRECTORY_FILE,
            ..create_request(r"\empty", CreateDisposition::FILE_OPEN)
        },
    );
    assert_eq!(response_status(&response), NtStatus::SUCCESS);

    close(&mut backend, response_file_id(&response));

    assert!(!fixture.share.join("empty").exists());
}

// oracle: spec MS-FSA §2.1.5.15.3, §2.1.5.15.12
#[rstest]
#[case::mark_for_deletion(delete_pending(true))]
#[case::rename(rename_to(r"\renamed.txt", false))]
fn a_handle_without_delete_access_can_neither_rename_nor_delete(#[case] set_buffer: FileInformationClass) {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doc.txt"), b"kept").expect("write file");
    let mut backend = fixture.backend();
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            desired_access: DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY
                | DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE,
            ..create_request(r"\doc.txt", CreateDisposition::FILE_OPEN)
        },
    );
    let file_id = response_file_id(&response);

    let response = set_information(&mut backend, file_id, set_buffer);
    close(&mut backend, file_id);

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert!(fixture.share.join("doc.txt").exists());
    assert!(!fixture.share.join("renamed.txt").exists());
}

/// Root writes any file, so only an unprivileged run, as on CI, reaches the read-only fallback.
// oracle: spec MS-DTYP §2.4.3
#[test]
fn maximum_allowed_opens_a_file_the_user_cannot_write_for_reading() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = Fixture::new();
    let path = fixture.share.join("locked.txt");
    std::fs::write(&path, b"locked").expect("write file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).expect("make file read-only");
    let mut backend = fixture.backend();
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            desired_access: DesiredAccess::MAXIMUM_ALLOWED,
            ..create_request(r"\locked.txt", CreateDisposition::FILE_OPEN)
        },
    );
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let file_id = response_file_id(&response);

    let response = read(&mut backend, file_id, 0, 6);
    close(&mut backend, file_id);

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
}

// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn a_read_only_file_cannot_be_marked_for_deletion() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = Fixture::new();
    let path = fixture.share.join("locked.txt");
    std::fs::write(&path, b"locked").expect("write file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).expect("make file read-only");
    let mut backend = fixture.backend();
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            desired_access: DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY | DesiredAccess::DELETE,
            ..create_request(r"\locked.txt", CreateDisposition::FILE_OPEN)
        },
    );
    let file_id = response_file_id(&response);

    let response = set_information(&mut backend, file_id, delete_pending(true));
    close(&mut backend, file_id);

    // oracle: spec MS-ERREF §2.3.1
    assert_eq!(response_status(&response), NtStatus::from(0xC000_0121));
    assert!(path.exists());
}

// oracle: spec MS-FSA §2.1.5.6.3
#[rstest]
#[case::star_suffix(r"\dir\*.txt", &["B.TXT", "a.txt"])]
#[case::dos_star_suffix(r"\dir\<.txt", &["B.TXT", "a.txt"])]
#[case::question_mark(r"\dir\?.doc", &["c.doc"])]
#[case::star_dot_star_includes_dot_entries(r"\dir\*.*", &[".", "..", "B.TXT", "a.txt", "c.doc"])]
#[case::dot_entries_follow_dot_into_a_listing(r"\dir\?", &[".", ".."])]
#[case::dot_entries_follow_dot_out_of_a_listing(r"\dir\*?.*", &["B.TXT", "a.txt", "c.doc"])]
#[case::literal_name(r"\dir\c.doc", &["c.doc"])]
fn query_directory_matches_wildcard_patterns(#[case] pattern: &str, #[case] expected: &[&str]) {
    let fixture = Fixture::new();
    let directory = fixture.share.join("dir");
    std::fs::create_dir(&directory).expect("create directory");
    for name in ["a.txt", "B.TXT", "c.doc"] {
        std::fs::write(directory.join(name), name).expect("write file");
    }
    let mut backend = fixture.backend();
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);

    let (mut names, status) = list_directory(&mut backend, directory_id, pattern);

    names.sort();
    assert_eq!(names, expected);
    assert_eq!(status, NtStatus::NO_MORE_FILES);
}

// oracle: spec MS-FSA §2.1.5.8
#[test]
fn an_exclusive_lock_refuses_a_lock_through_another_handle() {
    let (_fixture, mut backend, first_id, second_id) = two_handles();

    let first = lock(&mut backend, first_id, 1, LockOperation::Exclusive, false, 0, 10);
    let second = lock(&mut backend, second_id, 2, LockOperation::Shared, false, 5, 10);

    assert_eq!(response_status(&first), NtStatus::SUCCESS);
    assert_eq!(response_status(&second), NtStatus::LOCK_NOT_GRANTED);
}

// oracle: spec MS-FSA §2.1.5.3
#[test]
fn an_exclusive_lock_refuses_reads_through_another_handle() {
    let (_fixture, mut backend, first_id, second_id) = two_handles();
    let _ = lock(&mut backend, first_id, 1, LockOperation::Exclusive, false, 0, 10);

    let owner_read = read(&mut backend, first_id, 0, 4);
    let other_read = read(&mut backend, second_id, 0, 4);

    assert_eq!(response_status(&owner_read), NtStatus::SUCCESS);
    // oracle: spec MS-ERREF §2.3.1
    assert_eq!(response_status(&other_read), NtStatus::from(0xC000_0054));
}

/// Offset 5 lies inside the lock, where a zero-length range at any offset but 0 still overlaps it ([MS-FSA] 2.1.4.10).
// oracle: spec MS-FSA §2.1.5.3, §2.1.5.4
#[test]
fn zero_length_reads_and_writes_succeed_inside_another_handles_lock() {
    let (_fixture, mut backend, first_id, second_id) = two_handles();
    let _ = lock(&mut backend, first_id, 1, LockOperation::Exclusive, false, 0, 10);

    let read_response = read(&mut backend, second_id, 5, 0);
    let write_response = write(&mut backend, second_id, 5, b"");

    assert_eq!(response_status(&read_response), NtStatus::SUCCESS);
    assert_eq!(response_status(&write_response), NtStatus::SUCCESS);
}

// oracle: spec MS-FSA §2.1.4.10
#[test]
fn a_shared_lock_refuses_writes_even_through_its_owner() {
    let (_fixture, mut backend, first_id, _second_id) = two_handles();
    let _ = lock(&mut backend, first_id, 1, LockOperation::Shared, false, 0, 10);

    let response = write(&mut backend, first_id, 2, b"x");

    // oracle: spec MS-ERREF §2.3.1
    assert_eq!(response_status(&response), NtStatus::from(0xC000_0054));
}

/// Only the {0, 0} range is exempt; any other range of length 0 ends one byte before its offset.
// oracle: spec MS-FSA §2.1.4.10
#[rstest]
#[case::the_zero_range_against_the_zero_range((0, 0), (0, 0), NtStatus::SUCCESS)]
#[case::a_range_against_the_zero_range((0, 0), (0, 10), NtStatus::SUCCESS)]
#[case::length_zero_inside_a_lock((0, 10), (5, 0), NtStatus::LOCK_NOT_GRANTED)]
#[case::length_zero_at_the_start_of_a_lock((5, 10), (5, 0), NtStatus::SUCCESS)]
fn zero_length_ranges_overlap_by_their_last_byte(
    #[case] held: (u64, u64),
    #[case] requested: (u64, u64),
    #[case] status: NtStatus,
) {
    let (_fixture, mut backend, first_id, second_id) = two_handles();
    let first = lock(
        &mut backend,
        first_id,
        1,
        LockOperation::Exclusive,
        false,
        held.0,
        held.1,
    );

    let second = lock(
        &mut backend,
        second_id,
        2,
        LockOperation::Exclusive,
        false,
        requested.0,
        requested.1,
    );

    assert_eq!(response_status(&first), NtStatus::SUCCESS);
    assert_eq!(response_status(&second), status);
}

// oracle: spec MS-ERREF §2.3.1
#[rstest]
// oracle: spec MS-FSA §2.1.5.9
#[case::range_not_locked(LockOperation::Unlock, 0, 10, 0xC000_007E)]
// oracle: spec MS-FSA §2.1.5.8
#[case::range_past_the_largest_offset(LockOperation::Exclusive, u64::MAX, 2, 0xC000_001E)]
fn invalid_lock_requests_report_their_status(
    #[case] operation: LockOperation,
    #[case] offset: u64,
    #[case] length: u64,
    #[case] status: u32,
) {
    let (_fixture, mut backend, first_id, _second_id) = two_handles();

    let response = lock(&mut backend, first_id, 1, operation, false, offset, length);

    assert_eq!(response_status(&response), NtStatus::from(status));
}

// oracle: spec MS-FSA §2.1.5.8
#[test]
fn a_waiting_lock_is_granted_when_the_conflicting_lock_is_released() {
    let (_fixture, mut backend, first_id, second_id) = two_handles();
    let _ = lock(&mut backend, first_id, 1, LockOperation::Exclusive, false, 0, 10);

    let waiting = lock(&mut backend, second_id, 2, LockOperation::Exclusive, true, 0, 10);
    let released = lock(&mut backend, first_id, 3, LockOperation::Unlock, false, 0, 10);

    assert!(waiting.is_empty());
    assert_eq!(completions(&released), [(2, NtStatus::SUCCESS), (3, NtStatus::SUCCESS)]);
}

// oracle: spec MS-FSA §2.1.5.8
#[test]
fn closing_the_lock_holder_grants_a_waiting_lock() {
    let (_fixture, mut backend, first_id, second_id) = two_handles();
    let _ = lock(&mut backend, first_id, 1, LockOperation::Exclusive, false, 0, 10);
    let waiting = lock(&mut backend, second_id, 2, LockOperation::Exclusive, true, 0, 10);

    let responses = close(&mut backend, first_id);

    assert!(waiting.is_empty());
    // The close itself is answered with the completion ID of `request_header`.
    assert_eq!(
        completions(&responses),
        [(2, NtStatus::SUCCESS), (7, NtStatus::SUCCESS)]
    );
}

/// The server forgets a closed handle, so its waiting request would otherwise never be answered.
// oracle: mutant waiting lock left unanswered when its handle closes
#[test]
fn closing_a_handle_cancels_its_waiting_lock() {
    let (_fixture, mut backend, first_id, second_id) = two_handles();
    let _ = lock(&mut backend, first_id, 1, LockOperation::Exclusive, false, 0, 10);
    let _ = lock(&mut backend, second_id, 2, LockOperation::Exclusive, true, 0, 10);

    let responses = close(&mut backend, second_id);

    assert_eq!(
        completions(&responses),
        [(2, NtStatus::CANCELLED), (7, NtStatus::SUCCESS)]
    );
}

// oracle: spec MS-FSA §2.1.5.8
#[test]
fn a_directory_cannot_be_locked() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    let root_id = create(&mut backend, r"\", CreateDisposition::FILE_OPEN);

    let response = lock(&mut backend, root_id, 1, LockOperation::Exclusive, false, 0, 10);

    assert_eq!(response_status(&response), NtStatus::INVALID_PARAMETER);
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/backend.rs
#[test]
fn create_makes_the_file_in_the_root_of_its_drive() {
    let roots = TwoRoots::new();
    let mut backend = roots.backend();

    let _ = create_with(
        &mut backend,
        create_request_on(FIRST, r"\first.txt", CreateDisposition::FILE_CREATE),
    );
    let _ = create_with(
        &mut backend,
        create_request_on(SECOND, r"\second.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(names_in(&roots.first), ["first.txt"]);
    assert_eq!(names_in(&roots.second), ["second.txt"]);
}

/// Both roots hold a file of the same name, so a request resolved beneath the wrong root would still find one.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/backend.rs
#[test]
fn a_drive_reads_and_writes_the_file_in_its_own_root() {
    let roots = TwoRoots::new();
    std::fs::write(roots.first.join("doc.txt"), b"first").expect("write file");
    std::fs::write(roots.second.join("doc.txt"), b"second").expect("write file");
    let mut backend = roots.backend();
    let file_id = create_on(&mut backend, SECOND, r"\doc.txt", CreateDisposition::FILE_OPEN);

    let read = read_on(&mut backend, SECOND, file_id, 0, 16);
    let _ = write_on(&mut backend, SECOND, file_id, 0, b"SECOND");

    assert_eq!(response_read_data(&read), b"second");
    assert_eq!(std::fs::read(roots.first.join("doc.txt")).expect("read file"), b"first");
    assert_eq!(
        std::fs::read(roots.second.join("doc.txt")).expect("read file"),
        b"SECOND"
    );
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/backend.rs
#[test]
fn rename_stays_in_the_root_of_its_drive() {
    let roots = TwoRoots::new();
    std::fs::write(roots.first.join("doc.txt"), b"first").expect("write file");
    std::fs::write(roots.second.join("doc.txt"), b"second").expect("write file");
    let mut backend = roots.backend();
    let file_id = create_on(&mut backend, SECOND, r"\doc.txt", CreateDisposition::FILE_OPEN);

    let _ = set_information_on(&mut backend, SECOND, file_id, rename_to(r"\renamed.txt", false));

    assert_eq!(names_in(&roots.first), ["doc.txt"]);
    assert_eq!(names_in(&roots.second), ["renamed.txt"]);
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/backend.rs
#[test]
fn query_directory_lists_the_root_of_its_drive() {
    let roots = TwoRoots::new();
    std::fs::write(roots.first.join("first.txt"), b"first").expect("write file");
    std::fs::write(roots.second.join("second.txt"), b"second").expect("write file");
    let mut backend = roots.backend();
    let root_id = create_on(&mut backend, SECOND, r"\", CreateDisposition::FILE_OPEN);

    assert_eq!(
        list_directory_on(&mut backend, SECOND, root_id, r"\*"),
        (vec!["second.txt".to_owned()], NtStatus::NO_MORE_FILES)
    );
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/backend.rs
#[test]
fn a_delete_removes_the_file_in_the_root_of_its_drive() {
    let roots = TwoRoots::new();
    std::fs::write(roots.first.join("doc.txt"), b"first").expect("write file");
    std::fs::write(roots.second.join("doc.txt"), b"second").expect("write file");
    let mut backend = roots.backend();
    let file_id = create_on(&mut backend, SECOND, r"\doc.txt", CreateDisposition::FILE_OPEN);

    let _ = set_information_on(&mut backend, SECOND, file_id, delete_pending(true));
    close_on(&mut backend, SECOND, file_id);

    assert_eq!(names_in(&roots.first), ["doc.txt"]);
    assert!(names_in(&roots.second).is_empty());
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/backend.rs
#[test]
fn delete_on_close_removes_the_file_in_the_root_of_its_drive() {
    let roots = TwoRoots::new();
    std::fs::write(roots.first.join("doc.txt"), b"first").expect("write file");
    std::fs::write(roots.second.join("doc.txt"), b"second").expect("write file");
    let mut backend = roots.backend();
    let response = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options: CreateOptions::FILE_DELETE_ON_CLOSE,
            ..create_request_on(SECOND, r"\doc.txt", CreateDisposition::FILE_OPEN)
        },
    );
    assert_eq!(response_status(&response), NtStatus::SUCCESS);

    close_on(&mut backend, SECOND, response_file_id(&response));

    assert_eq!(names_in(&roots.first), ["doc.txt"]);
    assert!(names_in(&roots.second).is_empty());
}

/// Each root holds the file marked through the other drive, so a delete run beneath the wrong root would find a file to remove.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/backend.rs
#[test]
fn reset_runs_the_pending_deletes_of_every_drive_beneath_its_root() {
    let (roots, mut backend) = one_pending_delete_per_drive();

    backend.reset().expect("reset");

    assert_eq!(names_in(&roots.first), ["b.txt"]);
    assert_eq!(names_in(&roots.second), ["a.txt"]);
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/handles.rs
#[test]
fn dropping_the_backend_runs_the_pending_deletes_of_every_drive_beneath_its_root() {
    let (roots, backend) = one_pending_delete_per_drive();

    drop(backend);

    assert_eq!(names_in(&roots.first), ["b.txt"]);
    assert_eq!(names_in(&roots.second), ["a.txt"]);
}

/// Two roots holding `a.txt` and `b.txt` each, with `a.txt` marked for deletion through `FIRST` and `b.txt` through `SECOND`, and both handles left open.
fn one_pending_delete_per_drive() -> (TwoRoots, NixRdpdrBackend) {
    let roots = TwoRoots::new();
    for root in [&roots.first, &roots.second] {
        std::fs::write(root.join("a.txt"), b"a").expect("write file");
        std::fs::write(root.join("b.txt"), b"b").expect("write file");
    }
    let mut backend = roots.backend();
    for (device_id, path) in [(FIRST, r"\a.txt"), (SECOND, r"\b.txt")] {
        let file_id = create_on(&mut backend, device_id, path, CreateDisposition::FILE_OPEN);
        let response = set_information_on(&mut backend, device_id, file_id, delete_pending(true));
        assert_eq!(response_status(&response), NtStatus::SUCCESS);
    }
    (roots, backend)
}

/// A drive cannot reach the root of another drive, which lies next to its own.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/path.rs
#[rstest]
#[case::parent_directory(r"\..\second\secret.txt")]
#[case::symbolic_link_to_the_root(r"\link\secret.txt")]
#[case::symbolic_link_to_the_file(r"\file_link")]
fn a_drive_cannot_reach_the_root_of_another_drive(
    #[case] path: &str,
    #[values(CreateDisposition::FILE_OPEN, CreateDisposition::FILE_OVERWRITE_IF)] disposition: CreateDisposition,
) {
    let roots = TwoRoots::new();
    let secret = roots.second.join("secret.txt");
    std::fs::write(&secret, b"second").expect("write file");
    std::os::unix::fs::symlink(&roots.second, roots.first.join("link")).expect("link directory");
    std::os::unix::fs::symlink(&secret, roots.first.join("file_link")).expect("link file");
    let mut backend = roots.backend();

    let response = create_with(&mut backend, create_request_on(FIRST, path, disposition));

    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(std::fs::read(&secret).expect("read file"), b"second");
    assert_eq!(names_in(&roots.second), ["secret.txt"]);
}

/// A file ID names a file only through the device it was opened through.
/// Through another device it is answered like a file ID that names nothing, so the server learns nothing about the files of another drive, and the request changes nothing.
/// Each case gives the status a file ID that names nothing is answered with.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[rstest]
#[case::read(NtStatus::NO_SUCH_FILE, |file_id| ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
    device_io_request: header_on(SECOND, file_id, MajorFunction::Read),
    length: 4,
    offset: 8,
}))]
#[case::write(NtStatus::NO_SUCH_FILE, |file_id| ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
    device_io_request: header_on(SECOND, file_id, MajorFunction::Write),
    offset: 8,
    write_data: b"XXXX".to_vec(),
}))]
// This backend answers a close of a file ID that names nothing with success.
// oracle: mutant a close of a file ID that names nothing answered with another status
#[case::close(NtStatus::SUCCESS, |file_id| ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
    device_io_request: header_on(SECOND, file_id, MajorFunction::Close),
}))]
#[case::query_information(NtStatus::NO_SUCH_FILE, |file_id| {
    ServerDriveIoRequest::ServerDriveQueryInformationRequest(ServerDriveQueryInformationRequest {
        device_io_request: header_on(SECOND, file_id, MajorFunction::QueryInformation),
        file_info_class_lvl: FileInformationClassLevel::FILE_BASIC_INFORMATION,
    })
})]
#[case::query_volume_information(NtStatus::NO_SUCH_FILE, |file_id| {
    ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(ServerDriveQueryVolumeInformationRequest {
        device_io_request: header_on(SECOND, file_id, MajorFunction::QueryVolumeInformation),
        fs_info_class_lvl: FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION,
    })
})]
#[case::rename(NtStatus::NO_SUCH_FILE, |file_id| set_information_request(SECOND, file_id, rename_to(r"\moved.bin", false)))]
#[case::mark_for_deletion(NtStatus::NO_SUCH_FILE, |file_id| set_information_request(SECOND, file_id, delete_pending(true)))]
#[case::set_end_of_file(NtStatus::NO_SUCH_FILE, |file_id| {
    set_information_request(SECOND, file_id, FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 0 }))
})]
#[case::set_allocation(NtStatus::NO_SUCH_FILE, |file_id| {
    set_information_request(
        SECOND,
        file_id,
        FileInformationClass::Allocation(FileAllocationInformation { allocation_size: 0 }),
    )
})]
#[case::set_basic_information(NtStatus::NO_SUCH_FILE, |file_id| {
    set_information_request(
        SECOND,
        file_id,
        FileInformationClass::Basic(FileBasicInformation {
            creation_time: 0,
            last_access_time: 0,
            last_write_time: 0,
            change_time: 0,
            file_attributes: FileAttributes::FILE_ATTRIBUTE_READONLY,
        }),
    )
})]
#[case::initial_directory_query(NtStatus::NO_SUCH_FILE, |file_id| query_directory_request(SECOND, file_id, r"\*", true))]
#[case::continued_directory_query(NtStatus::NO_SUCH_FILE, |file_id| query_directory_request(SECOND, file_id, "", false))]
#[case::lock(NtStatus::NO_SUCH_FILE, |file_id| {
    lock_request(
        header_on(SECOND, file_id, MajorFunction::LockControl),
        LockOperation::Exclusive,
        false,
        8,
        4,
    )
})]
#[case::unlock(NtStatus::NO_SUCH_FILE, |file_id| {
    lock_request(
        header_on(SECOND, file_id, MajorFunction::LockControl),
        LockOperation::Unlock,
        false,
        0,
        4,
    )
})]
fn a_file_id_names_nothing_through_another_device(
    #[case] status: NtStatus,
    #[case] request: fn(u32) -> ServerDriveIoRequest,
) {
    let contents = b"0123456789abcdef";
    let roots = TwoRoots::new();
    std::fs::write(roots.first.join("data.bin"), contents).expect("write file");
    // The other root holds a link to the same file, so a request resolved beneath it would reach that file too.
    std::fs::hard_link(roots.first.join("data.bin"), roots.second.join("data.bin")).expect("link file");
    let mut backend = roots.backend();
    let file_id = create_on(&mut backend, FIRST, r"\data.bin", CreateDisposition::FILE_OPEN);
    let other_id = create_on(&mut backend, FIRST, r"\data.bin", CreateDisposition::FILE_OPEN);
    let locked = lock(&mut backend, file_id, 1, LockOperation::Exclusive, false, 0, 4);
    assert_eq!(response_status(&locked), NtStatus::SUCCESS);
    let unused_id = (0..)
        .find(|id| ![file_id, other_id].contains(id))
        .expect("an unused file ID");

    let unknown = send(&mut backend, request(unused_id));
    let refused = send(&mut backend, request(file_id));

    assert_eq!(encoded(&refused), encoded(&unknown));
    // oracle: mutant the status that answers a file ID that names nothing changed
    assert_eq!(response_status(&refused), status);
    // The handle is still open and still holds its lock, which refuses a read through another handle.
    // oracle: spec MS-FSA §2.1.5.3
    let other_read = read(&mut backend, other_id, 0, 4);
    assert_eq!(response_status(&other_read), NtStatus::from(0xC000_0054));
    // It holds no other lock, so another handle can lock the range the refused request named.
    // oracle: spec MS-FSA §2.1.5.8
    let other_lock = lock(&mut backend, other_id, 2, LockOperation::Exclusive, false, 8, 4);
    assert_eq!(response_status(&other_lock), NtStatus::SUCCESS);
    let own_read = read(&mut backend, file_id, 0, 4);
    assert_eq!(response_read_data(&own_read), b"0123");
    // Closing every handle deletes a file marked for deletion.
    // oracle: spec MS-FSA §2.1.5.5
    close(&mut backend, file_id);
    close(&mut backend, other_id);
    assert_eq!(names_in(&roots.first), ["data.bin"]);
    assert_eq!(names_in(&roots.second), ["data.bin"]);
    assert_eq!(
        std::fs::read(roots.first.join("data.bin")).expect("read file"),
        contents
    );
}

/// A query through another device neither restarts nor advances the enumeration a directory handle holds.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[test]
fn a_query_through_another_device_leaves_an_enumeration_alone() {
    let roots = TwoRoots::new();
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        std::fs::write(roots.first.join(name), name).expect("write file");
    }
    let mut backend = roots.backend();
    let root_id = create_on(&mut backend, FIRST, r"\", CreateDisposition::FILE_OPEN);
    // Two entries are read first, so a restarted enumeration shows as an entry listed twice.
    let first = query_directory_on(&mut backend, FIRST, root_id, r"\*", true);
    let second = query_directory_on(&mut backend, FIRST, root_id, "", false);
    assert_eq!(response_status(&second), NtStatus::SUCCESS);
    let mut names = vec![response_file_name(&first), response_file_name(&second)];

    let _ = query_directory_on(&mut backend, SECOND, root_id, "", false);
    let _ = query_directory_on(&mut backend, SECOND, root_id, r"\*", true);

    let mut response = query_directory_on(&mut backend, FIRST, root_id, "", false);
    while response_status(&response) == NtStatus::SUCCESS {
        names.push(response_file_name(&response));
        response = query_directory_on(&mut backend, FIRST, root_id, "", false);
    }
    names.sort();
    assert_eq!(names, ["a.txt", "b.txt", "c.txt", "d.txt"]);
}

/// Renaming one hard link leaves a handle opened through another link on that other link, on a drive with its own root as on the shared directory.
// oracle: spec MS-FSA §2.1.5.15.3
#[test]
fn renaming_a_hard_link_on_a_drive_with_its_own_root_does_not_move_handles_of_another_link() {
    let roots = TwoRoots::new();
    std::fs::write(roots.second.join("a.txt"), b"linked").expect("write file");
    std::fs::hard_link(roots.second.join("a.txt"), roots.second.join("b.txt")).expect("link file");
    let mut backend = roots.backend();
    let a_id = create_on(&mut backend, SECOND, r"\a.txt", CreateDisposition::FILE_OPEN);
    let b_id = create_on(&mut backend, SECOND, r"\b.txt", CreateDisposition::FILE_OPEN);

    let response = set_information_on(&mut backend, SECOND, a_id, rename_to(r"\c.txt", false));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let _ = set_information_on(&mut backend, SECOND, b_id, delete_pending(true));
    close_on(&mut backend, SECOND, a_id);
    close_on(&mut backend, SECOND, b_id);

    assert_eq!(names_in(&roots.second), ["c.txt"]);
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/factory.rs
#[test]
fn the_factory_builds_a_backend_that_serves_each_drive_from_its_root() {
    let roots = TwoRoots::new();
    let factory = NixRdpdrBackendFactory::default()
        .with_drive_root(FIRST, "first".to_owned(), utf8(&roots.first))
        .with_drive_root(SECOND, "second".to_owned(), utf8(&roots.second));

    let product = factory.build_rdpdr_backend().expect("valid configuration");

    let (mut backend, drives) = product.into_parts();
    let announced = drives
        .iter()
        .map(|drive| (drive.device_id(), drive.name()))
        .collect::<Vec<_>>();
    assert_eq!(announced, [(FIRST, "first"), (SECOND, "second")]);
    let backend = nix_backend(backend.as_mut());
    let _ = create_with(
        backend,
        create_request_on(FIRST, r"\first.txt", CreateDisposition::FILE_CREATE),
    );
    let _ = create_with(
        backend,
        create_request_on(SECOND, r"\second.txt", CreateDisposition::FILE_CREATE),
    );
    assert_eq!(names_in(&roots.first), ["first.txt"]);
    assert_eq!(names_in(&roots.second), ["second.txt"]);
}

/// A drive without its own root serves the shared directory, next to a drive that has one.
// oracle: mutant a drive with its own root moves the other drives off the shared directory
#[test]
fn a_drive_without_its_own_root_serves_the_shared_directory() {
    let roots = TwoRoots::new();
    let factory = NixRdpdrBackendFactory::new(utf8(&roots.first))
        .with_drive(FIRST, "shared".to_owned())
        .with_drive_root(SECOND, "own".to_owned(), utf8(&roots.second));
    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    let backend = nix_backend(backend.as_mut());

    let _ = create_with(
        backend,
        create_request_on(FIRST, r"\first.txt", CreateDisposition::FILE_CREATE),
    );
    let _ = create_with(
        backend,
        create_request_on(SECOND, r"\second.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(names_in(&roots.first), ["first.txt"]);
    assert_eq!(names_in(&roots.second), ["second.txt"]);
}

/// The factory does not open the roots, so the path given here is never used.
// oracle: mutant a drive with its own root skips the device ID and name checks
#[rstest]
#[case::device_id_zero(
    NixRdpdrBackendFactory::default().with_drive_root(0, "zero".to_owned(), "/".to_owned()),
    NixRdpdrBackendFactoryError::ReservedDeviceId
)]
#[case::device_id_of_another_drive_with_its_own_root(
    NixRdpdrBackendFactory::default()
        .with_drive_root(1, "a".to_owned(), "/".to_owned())
        .with_drive_root(1, "b".to_owned(), "/".to_owned()),
    NixRdpdrBackendFactoryError::DuplicateDeviceId(1)
)]
#[case::device_id_of_a_drive_of_the_shared_directory(
    NixRdpdrBackendFactory::new("/".to_owned())
        .with_drive(1, "a".to_owned())
        .with_drive_root(1, "b".to_owned(), "/".to_owned()),
    NixRdpdrBackendFactoryError::DuplicateDeviceId(1)
)]
#[case::empty_name(
    NixRdpdrBackendFactory::default().with_drive_root(1, String::new(), "/".to_owned()),
    NixRdpdrBackendFactoryError::InvalidDriveName(1)
)]
#[case::name_with_a_nul(
    NixRdpdrBackendFactory::default().with_drive_root(2, "home\0x".to_owned(), "/".to_owned()),
    NixRdpdrBackendFactoryError::InvalidDriveName(2)
)]
fn the_factory_rejects_an_invalid_drive_with_its_own_root(
    #[case] factory: NixRdpdrBackendFactory,
    #[case] expected: NixRdpdrBackendFactoryError,
) {
    let Err(error) = factory.build_rdpdr_backend() else {
        panic!("the factory accepted an invalid configuration");
    };

    assert_eq!(error.downcast_ref::<NixRdpdrBackendFactoryError>(), Some(&expected));
}

/// The printer avoids the device ID of a drive with its own root as it avoids the ID of a drive of the shared directory.
// oracle: mutant a drive with its own root left out of the device IDs the printer avoids
#[test]
fn the_printer_avoids_the_id_of_a_drive_with_its_own_root() {
    let factory =
        NixRdpdrBackendFactory::new("/".to_owned()).with_printer("printer".to_owned(), PrintTarget::DefaultPrinter);
    let free_id = printer_id(&factory);

    let beside_a_shared_drive = printer_id(&factory.clone().with_drive(free_id, "drive".to_owned()));
    let beside_a_rooted_drive = printer_id(&factory.with_drive_root(free_id, "drive".to_owned(), "/".to_owned()));

    assert_ne!(beside_a_rooted_drive, free_id);
    assert_eq!(beside_a_rooted_drive, beside_a_shared_drive);
    return;

    fn printer_id(factory: &NixRdpdrBackendFactory) -> u32 {
        let product = factory.build_rdpdr_backend().expect("valid configuration");
        product.printer().expect("a printer").device_id()
    }
}

/// A create through a device that has neither a root of its own nor a shared directory fails like one beneath a shared directory that cannot be opened, and makes no file.
// oracle: mutant a device without a root or a shared directory served the working directory or the root of another drive, or answered with another status
#[test]
fn create_fails_without_a_directory_to_serve() {
    let roots = TwoRoots::new();
    let mut unopenable = NixRdpdrBackend::new(utf8(&roots.first.join("missing")));
    let mut backend = NixRdpdrBackend::default().with_drive_root(SECOND, utf8(&roots.second));
    let name = format!("rdpdr-native-nix-{}-no-root.txt", std::process::id());
    let in_working_directory = std::env::current_dir().expect("working directory").join(&name);
    // The entry is absent before the request, so the removal after it can only remove what the request created.
    assert!(
        std::fs::symlink_metadata(&in_working_directory).is_err(),
        "{name} already exists in the working directory"
    );
    let request = || create_request_on(FIRST, &format!(r"\{name}"), CreateDisposition::FILE_CREATE);

    let response = create_with(&mut backend, request());

    let was_created = std::fs::symlink_metadata(&in_working_directory).is_ok();
    let _ = std::fs::remove_file(in_working_directory);
    assert!(!was_created);
    assert!(names_in(&roots.first).is_empty());
    assert!(names_in(&roots.second).is_empty());
    assert_eq!(
        response_status(&response),
        response_status(&create_with(&mut unopenable, request()))
    );
}

/// The lock requests waiting on every drive count against one limit, because one table holds the locks of every drive.
// oracle: mutant each drive given a lock table of its own
#[test]
fn drives_share_the_waiting_lock_limit() {
    let through_one_drive = waits_until_refused(&[FIRST]);

    let through_two_drives = waits_until_refused(&[FIRST, SECOND]);

    // The queue's own test in `src/nix/locks.rs`, `a_full_queue_refuses_another_waiting_request`, requires this status of a full queue.
    assert_eq!(through_one_drive.1, NtStatus::INSUFFICIENT_RESOURCES);
    assert!(0 < through_one_drive.0);
    assert_eq!(through_two_drives, through_one_drive);
    return;

    /// Locks a file of each device, then queues lock requests behind those locks, one device after the other, until one is refused.
    /// Returns the number of requests queued and the status of the refusal.
    fn waits_until_refused(devices: &[u32]) -> (usize, NtStatus) {
        let roots = TwoRoots::new();
        std::fs::write(roots.first.join("data.bin"), [0; 16]).expect("write file");
        std::fs::write(roots.second.join("data.bin"), [0; 16]).expect("write file");
        let mut backend = roots.backend();
        let mut waiters = Vec::new();
        for device_id in devices {
            let holder_id = create_on(&mut backend, *device_id, r"\data.bin", CreateDisposition::FILE_OPEN);
            let held = send(
                &mut backend,
                lock_request(
                    header_on(*device_id, holder_id, MajorFunction::LockControl),
                    LockOperation::Exclusive,
                    false,
                    0,
                    10,
                ),
            );
            assert_eq!(response_status(&held), NtStatus::SUCCESS);
            let waiter_id = create_on(&mut backend, *device_id, r"\data.bin", CreateDisposition::FILE_OPEN);
            waiters.push((*device_id, waiter_id));
        }
        for (queued, (device_id, waiter_id)) in waiters.iter().cycle().enumerate() {
            let response = send(
                &mut backend,
                lock_request(
                    header_on(*device_id, *waiter_id, MajorFunction::LockControl),
                    LockOperation::Exclusive,
                    true,
                    0,
                    10,
                ),
            );
            // A queued request is answered later.
            if !response.is_empty() {
                return (queued, response_status(&response));
            }
        }
        unreachable!("a cycle over the waiting handles does not end")
    }
}

/// A file is marked for deletion for every drive that serves it, whichever drive it was marked through.
// oracle: spec MS-FSA §2.1.5.5
#[rstest]
#[case::drives_of_the_shared_directory(drives_of_the_shared_directory)]
#[case::drives_with_the_same_root(drives_with_the_same_root)]
#[case::drives_with_the_same_root_under_two_spellings(drives_with_the_same_root_under_two_spellings)]
fn a_file_marked_for_deletion_through_one_drive_is_deleted_when_another_drive_closes_it_last(
    #[case] factory: fn(&str) -> NixRdpdrBackendFactory,
) {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("shared.txt"), b"shared").expect("write file");
    let factory = factory(&utf8(&fixture.share));
    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    let backend = nix_backend(backend.as_mut());
    let first_id = create_on(backend, FIRST, r"\shared.txt", CreateDisposition::FILE_OPEN);
    let second_id = create_on(backend, SECOND, r"\shared.txt", CreateDisposition::FILE_OPEN);
    let _ = set_information_on(backend, FIRST, first_id, delete_pending(true));

    let reopen = create_with(
        backend,
        create_request_on(SECOND, r"\shared.txt", CreateDisposition::FILE_OPEN),
    );
    close_on(backend, FIRST, first_id);
    let still_there = fixture.share.join("shared.txt").exists();
    close_on(backend, SECOND, second_id);

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
    assert_eq!(response_status(&reopen), NtStatus::DELETE_PENDING);
    assert!(still_there);
    assert!(!fixture.share.join("shared.txt").exists());
}

/// A handle follows a rename through a handle of another drive that serves the same directory.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
#[rstest]
#[case::drives_of_the_shared_directory(drives_of_the_shared_directory)]
#[case::drives_with_the_same_root(drives_with_the_same_root)]
#[case::drives_with_the_same_root_under_two_spellings(drives_with_the_same_root_under_two_spellings)]
fn delete_on_close_follows_a_rename_through_another_drive_of_the_same_root(
    #[case] factory: fn(&str) -> NixRdpdrBackendFactory,
) {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("doc.txt"), b"original").expect("write file");
    let factory = factory(&utf8(&fixture.share));
    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    let backend = nix_backend(backend.as_mut());
    let deleting_id = create_on(backend, FIRST, r"\doc.txt", CreateDisposition::FILE_OPEN);
    let renaming_id = create_on(backend, SECOND, r"\doc.txt", CreateDisposition::FILE_OPEN);

    let response = set_information_on(backend, SECOND, renaming_id, rename_to(r"\backup.tmp", false));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    std::fs::write(fixture.share.join("doc.txt"), b"replacement").expect("write replacement");
    let _ = set_information_on(backend, FIRST, deleting_id, delete_pending(true));
    close_on(backend, FIRST, deleting_id);
    close_on(backend, SECOND, renaming_id);

    assert_eq!(
        std::fs::read(fixture.share.join("doc.txt")).expect("read replacement"),
        b"replacement"
    );
    assert!(!fixture.share.join("backup.tmp").exists());
}

/// A drive serves the directory it was configured with, whatever a rename through an enclosing drive makes of its path.
// oracle: mutant a root opened again by path after an enclosing drive renamed a link into that path
#[test]
fn a_drive_keeps_its_directory_when_an_enclosing_drive_renames_a_link_into_its_path() {
    let roots = SwappedRoot::new();
    let mut backend = roots.backend();

    roots.swap_through_the_outer_drive(&mut backend);

    let file_id = create_on(&mut backend, SECOND, r"\inner.txt", CreateDisposition::FILE_OPEN);
    let read = read_on(&mut backend, SECOND, file_id, 0, 16);
    assert_eq!(response_read_data(&read), b"inner");
    let created = create_with(
        &mut backend,
        create_request_on(SECOND, r"\new.txt", CreateDisposition::FILE_CREATE),
    );
    assert_eq!(response_status(&created), NtStatus::SUCCESS);
    assert_eq!(names_in(&roots.outer.join("a_old").join("b")), ["inner.txt", "new.txt"]);
    assert_eq!(names_in(&roots.outside), ["outside.txt"]);
}

/// A backend built after a server renamed a link into the path of the inner root does not follow that link, so the inner drive is served nothing.
// oracle: mutant a link below a configured root followed when a root is resolved
#[test]
fn a_root_whose_path_a_server_led_through_a_link_serves_nothing() {
    let roots = SwappedRoot::new();
    let factory = roots.factory();
    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    roots.swap_through_the_outer_drive(nix_backend(backend.as_mut()));
    drop(backend);

    let (mut backend, drives) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    let backend = nix_backend(backend.as_mut());
    let through_the_inner_drive = create_with(
        backend,
        create_request_on(SECOND, r"\new.txt", CreateDisposition::FILE_CREATE),
    );
    let through_the_outer_drive = create_with(
        backend,
        create_request_on(FIRST, r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    let announced = drives.iter().map(|drive| drive.device_id()).collect::<Vec<_>>();
    assert_eq!(announced, [FIRST, SECOND]);
    assert_eq!(response_status(&through_the_inner_drive), NOTHING_SERVED);
    assert_eq!(names_in(&roots.outside), ["outside.txt"]);
    assert_eq!(response_status(&through_the_outer_drive), NtStatus::SUCCESS);
    assert!(roots.outer.join("new.txt").exists());
}

/// Pins a documented limitation: roots are resolved again for each backend, so one built after a server renamed another directory to the path of the inner root serves that directory through the inner drive.
/// That directory lies inside the outer root, where the outer drive reaches it as well.
// oracle: mutant a later backend refused a root whose path came to name another directory inside the enclosing root, or resolved it outside that root
#[test]
fn a_later_backend_serves_the_directory_a_server_renamed_to_the_path_of_the_inner_root() {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("inner.txt"), b"inner").expect("write file");
    let private = roots.outer.join("private").join("b");
    std::fs::create_dir_all(&private).expect("create directory");
    std::fs::write(private.join("private.txt"), b"private").expect("write file");
    let factory = NixRdpdrBackendFactory::default()
        .with_drive_root(FIRST, "outer".to_owned(), utf8(&roots.outer))
        .with_drive_root(SECOND, "inner".to_owned(), utf8(&roots.inner));
    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    rename_on(nix_backend(backend.as_mut()), FIRST, r"\a", r"\a_old");
    rename_on(nix_backend(backend.as_mut()), FIRST, r"\private", r"\a");
    drop(backend);

    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    let backend = nix_backend(backend.as_mut());
    let file_id = create_on(backend, SECOND, r"\private.txt", CreateDisposition::FILE_OPEN);
    let read = read_on(backend, SECOND, file_id, 0, 16);
    let created = create_with(
        backend,
        create_request_on(SECOND, r"\new.txt", CreateDisposition::FILE_CREATE),
    );
    let through_the_outer_drive = create_with(
        backend,
        create_request_on(FIRST, r"\a\b\new.txt", CreateDisposition::FILE_OPEN),
    );

    assert_eq!(response_read_data(&read), b"private");
    assert_eq!(response_status(&created), NtStatus::SUCCESS);
    assert_eq!(response_status(&through_the_outer_drive), NtStatus::SUCCESS);
    assert_eq!(names_in(&roots.outer.join("a").join("b")), ["new.txt", "private.txt"]);
    assert_eq!(names_in(&roots.outer.join("a_old").join("b")), ["inner.txt"]);
}

/// A root whose path follows a symbolic link in or below the directory of a configured root is served nothing, in whichever order the roots are given.
/// A server can replace such a link through the drive of that root, so the next backend would serve whatever the path then names.
/// Each case returns the roots of `FIRST`, which is served, and of `SECOND`, which is not.
// oracle: mutant a link in or below a configured root followed when a root is resolved
#[rstest]
#[case::a_link_directly_in_another_root(|fixture: &Fixture| {
    link(&fixture.sandbox.join("target"), &fixture.share.join("link"));
    [fixture.share.clone(), fixture.share.join("link")]
})]
#[case::a_link_below_another_root(|fixture: &Fixture| {
    std::fs::create_dir(fixture.share.join("sub")).expect("create directory");
    link(&fixture.sandbox.join("target"), &fixture.share.join("sub").join("link"));
    [fixture.share.clone(), fixture.share.join("sub").join("link")]
})]
#[case::a_link_outside_every_root_and_then_a_link_below_another_root(|fixture: &Fixture| {
    std::fs::create_dir(fixture.share.join("sub")).expect("create directory");
    link(&fixture.sandbox.join("target"), &fixture.share.join("sub").join("link"));
    link(&fixture.share.join("sub"), &fixture.sandbox.join("alias"));
    [fixture.share.clone(), fixture.sandbox.join("alias").join("link")]
})]
#[case::a_link_in_its_own_directory(|fixture: &Fixture| {
    link(&fixture.sandbox.join("target"), &fixture.sandbox.join("target").join("self"));
    [fixture.share.clone(), fixture.sandbox.join("target").join("self")]
})]
fn a_root_reached_through_a_link_inside_a_root_serves_nothing(
    #[case] roots: fn(&Fixture) -> [PathBuf; 2],
    #[values([FIRST, SECOND], [SECOND, FIRST])] order: [u32; 2],
) {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.sandbox.join("target")).expect("create directory");
    let roots = roots(&fixture);
    let mut backend = backend_with_roots_in_order(&roots, order);
    let before = entries_below(&fixture.sandbox);

    let refused = create_with(
        &mut backend,
        create_request_on(SECOND, r"\new.txt", CreateDisposition::FILE_CREATE),
    );
    let after = entries_below(&fixture.sandbox);
    let served = create_with(
        &mut backend,
        create_request_on(FIRST, r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(response_status(&refused), NOTHING_SERVED);
    assert_eq!(after, before);
    assert_eq!(response_status(&served), NtStatus::SUCCESS);
    assert!(fixture.share.join("new.txt").exists());
}

/// A root inside another root may be given through a symbolic link that lies outside every root, in whichever order the roots are given.
/// No drive reaches that link, so a server cannot change it.
// oracle: mutant a root refused for a link that lies in no configured root
#[rstest]
fn a_root_inside_another_may_be_given_through_a_link_outside_every_root(
    #[values([FIRST, SECOND], [SECOND, FIRST])] order: [u32; 2],
) {
    let nested = NestedRoots::new();
    let alias = nested.fixture.sandbox.join("alias");
    link(&nested.inner, &alias);
    let mut backend = backend_with_roots_in_order(&[nested.fixture.share.clone(), alias], order);

    let _ = create_with(
        &mut backend,
        create_request_on(SECOND, r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(names_in(&nested.inner), ["new.txt"]);
}

/// A root whose path enters the directory it names, descends, and comes back up through `..` is served nothing, from the first backend on.
/// A server that was served the directory could rename a link into the descent, so the `..` components would end somewhere else in a later backend: here in `outside`.
// oracle: mutant a `..` taken below a configured root left out of what makes a root unusable
#[test]
fn a_root_whose_path_comes_back_up_to_its_own_directory_serves_nothing() {
    let fixture = Fixture::new();
    let deeper = fixture.sandbox.join("outside").join("deep").join("deeper");
    for directory in [&fixture.share.join("p").join("sub"), &fixture.share.join("q"), &deeper] {
        std::fs::create_dir_all(directory).expect("create directory");
    }
    link(&deeper, &fixture.share.join("q").join("sub"));
    let alias = fixture.sandbox.join("alias");
    link(Path::new(&format!("{}/p/sub/../..", utf8(&fixture.share))), &alias);
    let factory = NixRdpdrBackendFactory::default().with_drive_root(FIRST, "alias".to_owned(), utf8(&alias));
    let before = entries_below(&fixture.sandbox);

    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    let renames = [(r"\p", r"\p_old"), (r"\q", r"\p")]
        .map(|(from, to)| try_rename_on(nix_backend(backend.as_mut()), FIRST, from, to));
    drop(backend);
    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    let created = create_with(
        nix_backend(backend.as_mut()),
        create_request_on(FIRST, r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(renames, [NOTHING_SERVED; 2]);
    assert_eq!(response_status(&created), NOTHING_SERVED);
    assert_eq!(entries_below(&fixture.sandbox), before);
}

/// Two roots whose paths each enter the directory of the other and leave it through `..` are both served nothing, in whichever order the roots are given.
/// Each case returns the target of the link that is the root of `FIRST`, which names the first directory through the second, and of the link that is the root of `SECOND`, the other way round.
// oracle: mutant a `..` taken in or below the directory of another root left out of what makes a root unusable
#[rstest]
#[case::directly_in_the_other_root(|roots: &TwoRoots| {
    [format!("{}/../share", utf8(&roots.second)), format!("{}/../second", utf8(&roots.first))]
})]
#[case::below_the_other_root(|roots: &TwoRoots| {
    [format!("{}/sub/../../share", utf8(&roots.second)), format!("{}/sub/../../second", utf8(&roots.first))]
})]
fn roots_whose_paths_leave_the_directory_of_each_other_by_a_parent_component_serve_nothing(
    #[case] targets: fn(&TwoRoots) -> [String; 2],
    #[values([FIRST, SECOND], [SECOND, FIRST])] order: [u32; 2],
) {
    let roots = TwoRoots::new();
    let sandbox = roots.second.parent().expect("a sandbox");
    let aliases = [sandbox.join("alias-first"), sandbox.join("alias-second")];
    for directory in [&roots.first, &roots.second] {
        std::fs::create_dir(directory.join("sub")).expect("create directory");
    }
    for (target, alias) in targets(&roots).iter().zip(&aliases) {
        link(Path::new(target), alias);
    }
    let mut backend = backend_with_roots_in_order(&aliases, order);
    let before = entries_below(sandbox);

    let through_each_drive = [FIRST, SECOND].map(|device_id| {
        let response = create_with(
            &mut backend,
            create_request_on(device_id, r"\new.txt", CreateDisposition::FILE_CREATE),
        );
        response_status(&response)
    });

    assert_eq!(through_each_drive, [NOTHING_SERVED; 2]);
    assert_eq!(entries_below(sandbox), before);
}

/// A root whose path does not resolve to a directory is served nothing, not even the shared directory, which the other drive is still served.
/// Opening the root creates nothing, so a root that was served would only show in the status.
// oracle: mutant a root path that must not resolve left to the kernel, which resolves it, or a drive with such a root served the shared directory
#[rstest]
// `src` exists in the package root, the working directory of the tests, so the path fails only for being relative.
#[case::a_relative_path(|_: &TwoRoots| "src".to_owned())]
// The path names the second root when walked from `/`, so it too fails only for being relative.
// oracle: reference crates/ironrdp-rdpdr-native/src/windows/factory.rs
#[case::a_relative_path_that_names_a_directory_from_the_filesystem_root(|roots: &TwoRoots| {
    utf8(&roots.second).trim_start_matches('/').to_owned()
})]
#[case::an_empty_path(|_: &TwoRoots| String::new())]
#[case::a_parent_component(|roots: &TwoRoots| format!("{}/../second", utf8(&roots.first)))]
#[case::a_current_directory_component(|roots: &TwoRoots| format!("{}/.", utf8(&roots.second)))]
#[case::a_missing_directory(|roots: &TwoRoots| utf8(&roots.second.join("missing")))]
#[case::a_file(|roots: &TwoRoots| {
    std::fs::write(roots.second.join("file.txt"), b"file").expect("write file");
    utf8(&roots.second.join("file.txt"))
})]
#[case::a_link_to_itself(|roots: &TwoRoots| {
    link(Path::new("loop"), &roots.second.join("loop"));
    utf8(&roots.second.join("loop"))
})]
fn a_root_that_names_no_directory_serves_nothing(#[case] root: fn(&TwoRoots) -> String) {
    let roots = TwoRoots::new();
    let mut backend = NixRdpdrBackend::new(utf8(&roots.first)).with_drive_root(SECOND, root(&roots));

    let unresolvable = create_with(
        &mut backend,
        create_request_on(SECOND, r"\", CreateDisposition::FILE_OPEN),
    );
    let served = create_with(
        &mut backend,
        create_request_on(FIRST, r"\", CreateDisposition::FILE_OPEN),
    );

    assert_eq!(response_status(&unresolvable), NOTHING_SERVED);
    assert_eq!(response_status(&served), NtStatus::SUCCESS);
}

/// Beside a drive with its own root, the shared directory is resolved like a root, so one that names no directory serves nothing to its drives.
// oracle: mutant the shared directory left to the kernel beside a drive with its own root
#[rstest]
#[case::a_missing_directory(|roots: &TwoRoots| utf8(&roots.first.join("missing")))]
// `src` exists in the package root, the working directory of the tests, so the path fails only for being relative.
#[case::a_relative_path(|_: &TwoRoots| "src".to_owned())]
fn a_shared_directory_that_names_no_directory_serves_nothing_beside_a_drive_with_its_own_root(
    #[case] shared: fn(&TwoRoots) -> String,
) {
    let roots = TwoRoots::new();
    let mut backend = NixRdpdrBackend::new(shared(&roots)).with_drive_root(SECOND, utf8(&roots.second));

    let unresolvable = create_with(
        &mut backend,
        create_request_on(FIRST, r"\", CreateDisposition::FILE_OPEN),
    );
    let served = create_with(
        &mut backend,
        create_request_on(SECOND, r"\", CreateDisposition::FILE_OPEN),
    );

    assert_eq!(response_status(&unresolvable), NOTHING_SERVED);
    assert_eq!(response_status(&served), NtStatus::SUCCESS);
}

/// A root whose path names no directory leaves the directory of no root undetermined, in whichever order the roots are given.
/// A root given through a symbolic link that lies outside every root is therefore served beside it, as beside a root that resolves.
/// Each case returns a path that the walk resolves on the filesystem and finds to name no directory.
// oracle: mutant a root path that names no directory made the roots reached through a link unusable
#[rstest]
// `src` exists in the package root, the working directory of the tests, so the path fails only for being relative.
#[case::a_relative_path(|_: &Fixture| PathBuf::from("src"))]
#[case::a_missing_directory(|fixture: &Fixture| fixture.sandbox.join("missing"))]
#[case::a_file(|fixture: &Fixture| {
    std::fs::write(fixture.sandbox.join("file.txt"), b"file").expect("write file");
    fixture.sandbox.join("file.txt")
})]
#[case::a_link_to_itself(|fixture: &Fixture| {
    link(Path::new("loop"), &fixture.sandbox.join("loop"));
    fixture.sandbox.join("loop")
})]
#[case::a_name_longer_than_a_directory_holds(|fixture: &Fixture| fixture.sandbox.join("n".repeat(256)))]
fn a_root_given_through_a_link_outside_every_root_is_served_beside_a_root_that_names_no_directory(
    #[case] root: fn(&Fixture) -> PathBuf,
    #[values([FIRST, SECOND], [SECOND, FIRST])] order: [u32; 2],
) {
    let fixture = Fixture::new();
    let alias = fixture.sandbox.join("alias");
    link(&fixture.share, &alias);
    let mut backend = backend_with_roots_in_order(&[alias, root(&fixture)], order);

    let through_the_root_that_names_no_directory = create_with(
        &mut backend,
        create_request_on(SECOND, r"\", CreateDisposition::FILE_OPEN),
    );
    let _ = create_with(
        &mut backend,
        create_request_on(FIRST, r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(
        response_status(&through_the_root_that_names_no_directory),
        NOTHING_SERVED
    );
    assert_eq!(names_in(&fixture.share), ["new.txt"]);
}

/// A root that may not be read names no directory a drive could serve, so it too leaves a root given through a link outside every root served.
/// Run as a user that no permission stops, the test proves nothing.
// oracle: mutant a root that may not be read counted as a directory that could not be determined
#[test]
fn a_root_given_through_a_link_outside_every_root_is_served_beside_a_root_that_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = Fixture::new();
    let (alias, unreadable) = (fixture.sandbox.join("alias"), fixture.sandbox.join("unreadable"));
    link(&fixture.share, &alias);
    std::fs::create_dir(&unreadable).expect("create directory");
    let set_mode = |mode| {
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(mode)).expect("set permissions");
    };

    set_mode(0o300);
    let mut backend = NixRdpdrBackend::default()
        .with_drive_root(FIRST, utf8(&alias))
        .with_drive_root(SECOND, utf8(&unreadable));
    // The fixture lists the directory to remove it.
    set_mode(0o700);

    let _ = create_with(
        &mut backend,
        create_request(r"\new.txt", CreateDisposition::FILE_CREATE),
    );
    assert_eq!(names_in(&fixture.share), ["new.txt"]);
}

/// Beside a drive with its own root, the shared directory is checked like a root: neither may be reached through a link inside the other.
/// Each case returns the shared directory, which `FIRST` serves, and the root of `SECOND`, with the device that is served nothing.
// oracle: mutant the shared directory left out of the roots a followed link is checked against
#[rstest]
#[case::the_shared_directory_through_a_link_in_the_root(|fixture: &Fixture| {
    link(&fixture.sandbox.join("target"), &fixture.share.join("link"));
    ([fixture.share.join("link"), fixture.share.clone()], FIRST)
})]
#[case::the_root_through_a_link_in_the_shared_directory(|fixture: &Fixture| {
    link(&fixture.sandbox.join("target"), &fixture.share.join("link"));
    ([fixture.share.clone(), fixture.share.join("link")], SECOND)
})]
fn a_link_between_the_shared_directory_and_a_root_is_not_followed(#[case] roots: fn(&Fixture) -> ([PathBuf; 2], u32)) {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.sandbox.join("target")).expect("create directory");
    let ([shared, root], refused) = roots(&fixture);
    let mut backend = NixRdpdrBackend::new(utf8(&shared)).with_drive_root(SECOND, utf8(&root));

    let response = create_with(
        &mut backend,
        create_request_on(refused, r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(response_status(&response), NOTHING_SERVED);
    assert!(names_in(&fixture.sandbox.join("target")).is_empty());
}

/// Beside a drive with its own root, the shared directory is held like a root, so its drives keep serving it after its path comes to name another directory.
// oracle: mutant the shared directory opened by path for each request beside a drive with its own root
#[test]
fn the_shared_directory_is_held_beside_a_drive_with_its_own_root() {
    let roots = TwoRoots::new();
    let sandbox = roots.first.parent().expect("a sandbox");
    let moved = sandbox.join("moved");
    let mut backend = NixRdpdrBackend::new(utf8(&roots.first)).with_drive_root(SECOND, utf8(&roots.second));

    std::fs::rename(&roots.first, &moved).expect("move shared directory");
    std::fs::create_dir(&roots.first).expect("create directory");
    let _ = create_with(
        &mut backend,
        create_request_on(FIRST, r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(names_in(&moved), ["new.txt"]);
    assert!(names_in(&roots.first).is_empty());
}

/// A backend whose drives all serve the shared directory opens it by path for each request, so it serves a directory created after it was built.
/// Until then a create fails with STATUS_UNSUCCESSFUL, the status of a shared directory that cannot be opened.
// oracle: mutant the shared directory of a backend without a drive root resolved when the backend is built
#[test]
fn a_shared_directory_alone_is_opened_for_each_request() {
    let fixture = Fixture::new();
    let late = fixture.sandbox.join("late");
    let mut backend = NixRdpdrBackend::new(utf8(&late));

    let before = create_with(
        &mut backend,
        create_request(r"\new.txt", CreateDisposition::FILE_CREATE),
    );
    std::fs::create_dir(&late).expect("create shared directory");
    let after = create_with(
        &mut backend,
        create_request(r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(response_status(&before), NtStatus::UNSUCCESSFUL);
    assert_eq!(response_status(&after), NtStatus::SUCCESS);
    assert_eq!(names_in(&late), ["new.txt"]);
}

/// A backend whose drives all serve the shared directory leaves its path to the kernel, so paths a drive root may not have are served.
// oracle: mutant the shared directory of a backend without a drive root held to the rules for a drive root
#[rstest]
#[case::a_relative_path(|_: &Fixture| ".".to_owned())]
#[case::a_parent_component(|fixture: &Fixture| format!("{}/../share", utf8(&fixture.share)))]
#[case::a_link_in_the_shared_directory(|fixture: &Fixture| {
    link(Path::new("."), &fixture.share.join("self"));
    utf8(&fixture.share.join("self"))
})]
fn a_shared_directory_alone_may_have_any_path(#[case] shared: fn(&Fixture) -> String) {
    let fixture = Fixture::new();
    let mut backend = NixRdpdrBackend::new(shared(&fixture));

    let response = create_with(&mut backend, create_request(r"\", CreateDisposition::FILE_OPEN));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
}

/// The directories leading to a root need to be searched, not read, as for any path the kernel resolves.
/// Run as a user that no permission stops, the test proves nothing.
// oracle: mutant the directories leading to a root opened for reading
#[test]
fn a_root_beneath_a_directory_that_cannot_be_read_is_served() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = Fixture::new();
    let unreadable = fixture.sandbox.join("unreadable");
    std::fs::create_dir_all(unreadable.join("root")).expect("create root");
    let set_mode = |mode| {
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(mode)).expect("set permissions");
    };

    set_mode(0o300);
    let mut backend = NixRdpdrBackend::default().with_drive_root(FIRST, utf8(&unreadable.join("root")));
    // The fixture lists the directory to remove it.
    set_mode(0o700);

    let response = create_with(&mut backend, create_request(r"\", CreateDisposition::FILE_OPEN));
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
}

/// A `..` in the target of a followed link leads to the parent of the directory reached, not of the directory holding the link.
/// Here the two differ, because the target first passes through another link.
// oracle: spec POSIX.1-2017 §4.13
#[test]
fn a_parent_component_in_a_link_target_leads_to_the_parent_of_the_directory_reached() {
    let fixture = Fixture::new();
    let (links, deep) = (fixture.sandbox.join("links"), fixture.sandbox.join("deep"));
    for directory in [links.join("kept"), deep.join("inner"), deep.join("kept")] {
        std::fs::create_dir_all(directory).expect("create directory");
    }
    link(&deep.join("inner"), &links.join("jump"));
    link(Path::new("jump/../kept"), &links.join("root"));
    let mut backend = NixRdpdrBackend::default().with_drive_root(FIRST, utf8(&links.join("root")));

    let _ = create_with(
        &mut backend,
        create_request(r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(names_in(&deep.join("kept")), ["new.txt"]);
    assert!(names_in(&links.join("kept")).is_empty());
}

/// A `..` at `/` in the target of a followed link stays at `/`, as when the kernel resolves the path.
/// The test lists the root through the same link, so the kernel decides where the file must be.
// oracle: reference the kernel's resolution of the same link
#[test]
fn a_parent_component_at_the_filesystem_root_stays_there() {
    let fixture = Fixture::new();
    let alias = fixture.sandbox.join("alias");
    link(Path::new(&format!("/..{}", utf8(&fixture.share))), &alias);
    let mut backend = NixRdpdrBackend::default().with_drive_root(FIRST, utf8(&alias));

    let _ = create_with(
        &mut backend,
        create_request(r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(names_in(&alias), ["new.txt"]);
}

/// Resolving a root is bounded, so a path built to make it run long names no directory, although every link in it leads on to one.
// oracle: mutant a bound on the links followed or the components walked removed
#[rstest]
#[case::a_chain_of_links(|fixture: &Fixture| {
    for index in 0..64 {
        link(Path::new(&format!("chain{}", index + 1)), &fixture.sandbox.join(format!("chain{index}")));
    }
    link(&fixture.share, &fixture.sandbox.join("chain64"));
    fixture.sandbox.join("chain0")
})]
#[case::links_of_many_components(|fixture: &Fixture| {
    std::fs::create_dir(fixture.sandbox.join("d")).expect("create directory");
    let detour = "d/../".repeat(150);
    for index in 0..4 {
        link(Path::new(&format!("{detour}detour{}", index + 1)), &fixture.sandbox.join(format!("detour{index}")));
    }
    link(&fixture.share, &fixture.sandbox.join("detour4"));
    fixture.sandbox.join("detour0")
})]
fn a_root_path_that_takes_too_long_to_resolve_serves_nothing(#[case] root: fn(&Fixture) -> PathBuf) {
    let fixture = Fixture::new();
    let mut backend = NixRdpdrBackend::default().with_drive_root(FIRST, utf8(&root(&fixture)));

    let response = create_with(&mut backend, create_request(r"\", CreateDisposition::FILE_OPEN));

    assert_eq!(response_status(&response), NOTHING_SERVED);
}

/// A root given again for the same device replaces the earlier one, which then no longer counts among the roots a followed link is checked against.
/// The root of `SECOND` lies behind a link in the first root given to `FIRST`, and in no root once that one is replaced.
// oracle: mutant the earlier root of a device still served, or still checked against
#[test]
fn a_root_given_again_replaces_the_earlier_one() {
    let fixture = Fixture::new();
    let (target, other) = (fixture.sandbox.join("target"), fixture.sandbox.join("other"));
    std::fs::create_dir(&target).expect("create directory");
    std::fs::create_dir(&other).expect("create directory");
    link(&target, &fixture.share.join("link"));
    let mut backend = NixRdpdrBackend::default()
        .with_drive_root(FIRST, utf8(&fixture.share))
        .with_drive_root(SECOND, utf8(&fixture.share.join("link")))
        .with_drive_root(FIRST, utf8(&other));

    let _ = create_with(
        &mut backend,
        create_request_on(FIRST, r"\first.txt", CreateDisposition::FILE_CREATE),
    );
    let _ = create_with(
        &mut backend,
        create_request_on(SECOND, r"\second.txt", CreateDisposition::FILE_CREATE),
    );

    assert_eq!(names_in(&other), ["first.txt"]);
    assert_eq!(names_in(&target), ["second.txt"]);
    assert_eq!(names_in(&fixture.share), ["link"]);
}

/// A listing of the root of a drive starts at the first entry each time, although the drive holds its root open between requests.
// oracle: spec MS-FSA §2.1.5.6.3
#[test]
fn the_root_of_a_drive_lists_from_the_start_each_time() {
    let roots = TwoRoots::new();
    std::fs::write(roots.second.join("only.txt"), b"only").expect("write file");
    let mut backend = roots.backend();
    let root_id = create_on(&mut backend, SECOND, r"\", CreateDisposition::FILE_OPEN);
    let listed = (vec!["only.txt".to_owned()], NtStatus::NO_MORE_FILES);

    assert_eq!(list_directory_on(&mut backend, SECOND, root_id, r"\*"), listed);
    assert_eq!(list_directory_on(&mut backend, SECOND, root_id, r"\*"), listed);
}

/// A drive serves the directory it was configured with after a local change makes its path name something else.
/// The channel restarting does not resolve the path again either.
// oracle: mutant a root opened again by path, for a request or when the channel restarts
#[rstest]
#[case::a_link_to_another_directory(|other: &Path, path: &Path| {
    std::os::unix::fs::symlink(other, path).expect("link directory");
})]
#[case::another_directory(|other: &Path, path: &Path| std::fs::rename(other, path).expect("move directory"))]
fn a_drive_keeps_its_directory_when_its_path_comes_to_name_another(#[case] replace_path: fn(&Path, &Path)) {
    let roots = TwoRoots::new();
    std::fs::write(roots.second.join("kept.txt"), b"kept").expect("write file");
    let sandbox = roots.second.parent().expect("a sandbox");
    let (moved, other) = (sandbox.join("moved"), sandbox.join("other"));
    std::fs::create_dir(&other).expect("create directory");
    std::fs::write(other.join("kept.txt"), b"other").expect("write file");
    let mut backend = roots.backend();

    std::fs::rename(&roots.second, &moved).expect("move root");
    replace_path(&other, &roots.second);
    let before_reset = read_kept_and_create(&mut backend, r"\before.txt");
    backend.reset().expect("reset");
    let after_reset = read_kept_and_create(&mut backend, r"\after.txt");

    assert_eq!(before_reset, b"kept");
    assert_eq!(after_reset, b"kept");
    assert_eq!(names_in(&moved), ["after.txt", "before.txt", "kept.txt"]);
    assert_eq!(names_in(&roots.second), ["kept.txt"]);
    return;

    /// Reads `kept.txt` through `SECOND` and creates `path` through it.
    fn read_kept_and_create(backend: &mut NixRdpdrBackend, path: &str) -> Vec<u8> {
        let file_id = create_on(backend, SECOND, r"\kept.txt", CreateDisposition::FILE_OPEN);
        let read = read_on(backend, SECOND, file_id, 0, 16);
        let _ = create_with(backend, create_request_on(SECOND, path, CreateDisposition::FILE_CREATE));
        response_read_data(&read)
    }
}

/// An exclusive lock taken through one drive refuses a read through the other drive of the same file, whichever drive is the enclosing one.
// oracle: spec MS-FSA §2.1.5.3
#[rstest]
fn an_exclusive_lock_refuses_a_read_through_a_drive_nested_with_the_locking_one(
    #[values([FIRST, SECOND], [SECOND, FIRST])] devices: [u32; 2],
) {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("data.bin"), [0; 16]).expect("write file");
    let mut backend = roots.backend();
    let [locking, reading] = devices;
    let [locking_id, reading_id] = roots.open_through(&mut backend, devices, "data.bin");

    let locked = send(
        &mut backend,
        lock_request(
            header_on(locking, locking_id, MajorFunction::LockControl),
            LockOperation::Exclusive,
            false,
            0,
            10,
        ),
    );
    let read = read_on(&mut backend, reading, reading_id, 0, 4);

    assert_eq!(response_status(&locked), NtStatus::SUCCESS);
    // oracle: spec MS-ERREF §2.3.1
    assert_eq!(response_status(&read), NtStatus::from(0xC000_0054));
}

/// A lock request waiting through one drive is granted when the conflicting lock taken through the other drive of the same file is released, whichever drive is the enclosing one.
// oracle: spec MS-FSA §2.1.5.8
#[rstest]
fn a_waiting_lock_is_granted_when_a_drive_nested_with_the_waiting_one_releases_the_conflicting_lock(
    #[values([FIRST, SECOND], [SECOND, FIRST])] devices: [u32; 2],
) {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("data.bin"), [0; 16]).expect("write file");
    let mut backend = roots.backend();
    let [holding, waiting] = devices;
    let [holding_id, waiting_id] = roots.open_through(&mut backend, devices, "data.bin");
    let _ = send(
        &mut backend,
        lock_request_with(holding, holding_id, 1, LockOperation::Exclusive, false),
    );

    let waited = send(
        &mut backend,
        lock_request_with(waiting, waiting_id, 2, LockOperation::Exclusive, true),
    );
    let released = send(
        &mut backend,
        lock_request_with(holding, holding_id, 3, LockOperation::Unlock, false),
    );

    assert!(waited.is_empty());
    assert_eq!(completions(&released), [(2, NtStatus::SUCCESS), (3, NtStatus::SUCCESS)]);
    return;

    /// A request on the first ten bytes of `file_id`, answered with `completion_id`.
    fn lock_request_with(
        device_id: u32,
        file_id: u32,
        completion_id: u32,
        operation: LockOperation,
        wait: bool,
    ) -> ServerDriveIoRequest {
        lock_request(
            DeviceIoRequest {
                completion_id,
                ..header_on(device_id, file_id, MajorFunction::LockControl)
            },
            operation,
            wait,
            0,
            10,
        )
    }
}

/// A file marked for deletion through one drive is marked for the other drive that reaches it too, whichever drive is the enclosing one.
/// The other drive cannot open it, with the status that refuses the same open on one drive, and it is removed when its last handle closes.
// oracle: spec MS-FSA §2.1.5.5
#[rstest]
fn a_file_marked_for_deletion_is_marked_for_a_drive_nested_with_the_marking_one(
    #[values([FIRST, SECOND], [SECOND, FIRST])] devices: [u32; 2],
) {
    let roots = DeepRoots::new();
    let file = roots.inner.join("doc.txt");
    std::fs::write(&file, b"doc").expect("write file");
    let mut backend = roots.backend();
    let [marking, other] = devices;
    let [marking_id, other_id] = roots.open_through(&mut backend, devices, "doc.txt");
    let marked = set_information_on(&mut backend, marking, marking_id, delete_pending(true));

    let reopen = create_with(
        &mut backend,
        create_request_on(
            other,
            &roots.path_through(other, "doc.txt"),
            CreateDisposition::FILE_OPEN,
        ),
    );
    close_on(&mut backend, marking, marking_id);
    let still_there = file.exists();
    close_on(&mut backend, other, other_id);

    assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    assert_eq!(response_status(&reopen), NtStatus::DELETE_PENDING);
    assert!(still_there);
    assert!(!file.exists());
}

/// The directories that led to a file when it was marked for deletion cannot be renamed while the delete is pending, because the file is deleted by the name it was marked through.
/// The handle that marked the file is closed here, and the handle left belongs to the inner drive, which keeps only the directories from its own root down from being renamed.
/// Each case marks the file through the outer drive and closes the handle it used.
// oracle: spec MS-FSA §2.1.5.15.12, §2.1.5.5
#[rstest]
#[case::marked_through_its_disposition(|backend: &mut NixRdpdrBackend| {
    let file_id = create_on(backend, FIRST, r"\a\b\f.txt", CreateDisposition::FILE_OPEN);
    let _ = set_information_on(backend, FIRST, file_id, delete_pending(true));
    close_on(backend, FIRST, file_id);
})]
#[case::marked_when_a_handle_opened_to_delete_on_close_closed(|backend: &mut NixRdpdrBackend| {
    let opened = create_with(
        backend,
        DeviceCreateRequest {
            create_options: CreateOptions::FILE_DELETE_ON_CLOSE,
            ..create_request_on(FIRST, r"\a\b\f.txt", CreateDisposition::FILE_OPEN)
        },
    );
    close_on(backend, FIRST, response_file_id(&opened));
})]
fn a_directory_on_the_path_of_a_pending_delete_cannot_be_renamed(#[case] mark_and_close: fn(&mut NixRdpdrBackend)) {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("f.txt"), b"marked").expect("write file");
    let mut backend = roots.backend();
    let inner_id = create_on(&mut backend, SECOND, r"\f.txt", CreateDisposition::FILE_OPEN);
    mark_and_close(&mut backend);
    let directory_id = create_on(&mut backend, FIRST, r"\a", CreateDisposition::FILE_OPEN);

    let response = set_information_on(&mut backend, FIRST, directory_id, rename_to(r"\moved", false));
    close_on(&mut backend, SECOND, inner_id);

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert_eq!(names_in(&roots.outer), ["a"]);
    assert_eq!(names_in(&roots.inner), Vec::<String>::new());
}

/// The directories that led to every name a file was marked for deletion through cannot be renamed while the delete is pending, whichever name was marked last.
/// The file has one hard link in `x`, marked through the outer drive, and one in `sub` beneath the inner root, marked through the inner drive.
/// Both marking handles are closed, and the handle left is on a third link in the outer root, which keeps neither directory from being renamed.
// oracle: mutant a directory on the path of a name other than the one marked last renamed while the delete is pending
#[rstest]
#[case::on_the_path_of_the_name_marked_through_the_outer_drive(r"\x")]
#[case::on_the_path_of_the_name_marked_through_the_inner_drive(r"\a\b\sub")]
fn a_directory_on_the_path_of_any_name_of_a_pending_delete_cannot_be_renamed(
    #[values(
        [(FIRST, r"\x\f.txt"), (SECOND, r"\sub\f.txt")],
        [(SECOND, r"\sub\f.txt"), (FIRST, r"\x\f.txt")]
    )]
    marks: [(u32, &str); 2],
    #[case] directory: &str,
) {
    let roots = DeepRoots::new();
    std::fs::create_dir(roots.outer.join("x")).expect("create directory");
    std::fs::create_dir(roots.inner.join("sub")).expect("create directory");
    std::fs::write(roots.outer.join("kept.txt"), b"marked").expect("write file");
    for link in [roots.outer.join("x"), roots.inner.join("sub")] {
        std::fs::hard_link(roots.outer.join("kept.txt"), link.join("f.txt")).expect("link file");
    }
    let mut backend = roots.backend();
    let _kept_id = create_on(&mut backend, FIRST, r"\kept.txt", CreateDisposition::FILE_OPEN);
    let handles = marks.map(|(device_id, path)| {
        (
            device_id,
            create_on(&mut backend, device_id, path, CreateDisposition::FILE_OPEN),
        )
    });
    for (device_id, file_id) in handles {
        let marked = set_information_on(&mut backend, device_id, file_id, delete_pending(true));
        assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    }
    for (device_id, file_id) in handles {
        close_on(&mut backend, device_id, file_id);
    }
    let directory_id = create_on(&mut backend, FIRST, directory, CreateDisposition::FILE_OPEN);
    let before = entries_below(&roots.outer);

    let response = set_information_on(&mut backend, FIRST, directory_id, rename_to(r"\moved", false));

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert_eq!(entries_below(&roots.outer), before);
}

/// A file is deleted by its name beneath the root of the drive it was marked through, so a directory above that root leads to no pending delete.
/// The enclosing drive can rename it, and the file is still deleted when its last handle closes.
// oracle: mutant a directory above the root of the drive a file was marked through kept from being renamed for the pending delete
#[test]
fn a_directory_above_the_root_of_the_marking_drive_can_be_renamed_while_the_delete_is_pending() {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("f.txt"), b"marked").expect("write file");
    let mut backend = roots.backend();
    let inner_id = create_on(&mut backend, SECOND, r"\f.txt", CreateDisposition::FILE_OPEN);
    let _ = set_information_on(&mut backend, SECOND, inner_id, delete_pending(true));
    let directory_id = create_on(&mut backend, FIRST, r"\a", CreateDisposition::FILE_OPEN);

    let response = set_information_on(&mut backend, FIRST, directory_id, rename_to(r"\moved", false));
    close_on(&mut backend, SECOND, inner_id);

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    // oracle: spec MS-FSA §2.1.5.5
    assert_eq!(names_in(&roots.outer.join("moved").join("b")), Vec::<String>::new());
}

/// A backend with only a shared directory decides a rename by its open handles alone, not by the pending deletes.
/// The handle left here is on another hard link of the file marked for deletion, so the directory that led to the marked link holds no open file, and is renamed.
// oracle: mutant a backend with only a shared directory refused to rename a directory for a pending delete below it
#[test]
fn a_directory_on_the_path_of_a_pending_delete_can_be_renamed_with_only_a_shared_directory() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("dir")).expect("create directory");
    std::fs::write(fixture.share.join("dir").join("f.txt"), b"marked").expect("write file");
    std::fs::hard_link(fixture.share.join("dir").join("f.txt"), fixture.share.join("link.txt")).expect("link file");
    let mut backend = fixture.backend();
    let _link_id = create(&mut backend, r"\link.txt", CreateDisposition::FILE_OPEN);
    let marked_id = create(&mut backend, r"\dir\f.txt", CreateDisposition::FILE_OPEN);
    let _ = set_information(&mut backend, marked_id, delete_pending(true));
    close(&mut backend, marked_id);
    let directory_id = create(&mut backend, r"\dir", CreateDisposition::FILE_OPEN);

    let response = set_information(&mut backend, directory_id, rename_to(r"\moved", false));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(names_in(&fixture.share), ["link.txt", "moved"]);
}

/// A file marked for deletion through both drives that reach it is deleted when its last handle closes, whichever drive marked it last.
/// A local process renames a directory above the inner root, so the name marked through the outer drive names nothing, while the name marked through the inner drive still names the file beneath the root that drive holds.
// oracle: spec MS-FSA §2.1.5.5
#[rstest]
fn a_file_marked_through_both_nested_drives_is_deleted_after_a_local_rename_above_the_inner_root(
    #[values([FIRST, SECOND], [SECOND, FIRST])] marking_order: [u32; 2],
) {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("f.txt"), b"marked").expect("write file");
    let mut backend = roots.backend();
    let handles = marking_order
        .into_iter()
        .zip(roots.open_through(&mut backend, marking_order, "f.txt"));
    for (device_id, file_id) in handles.clone() {
        let marked = set_information_on(&mut backend, device_id, file_id, delete_pending(true));
        assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    }
    std::fs::rename(roots.outer.join("a"), roots.outer.join("z")).expect("rename locally");

    for (device_id, file_id) in handles {
        close_on(&mut backend, device_id, file_id);
    }

    assert_eq!(names_in(&roots.outer.join("z").join("b")), Vec::<String>::new());
}

/// A file opened to delete on close through both drives that reach it is deleted when its last handle closes, although the handle that closes last marks a name that names nothing.
/// A local process renames a directory above the inner root before the closes, and the name the inner drive marked when its handle closed still names the file beneath the root that drive holds.
// oracle: spec MS-FSA §2.1.5.5
#[test]
fn a_file_opened_to_delete_on_close_through_both_nested_drives_is_deleted_after_a_local_rename_above_the_inner_root() {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("f.txt"), b"marked").expect("write file");
    let mut backend = roots.backend();
    let [inner_id, outer_id] = [SECOND, FIRST].map(|device_id| {
        let opened = create_with(
            &mut backend,
            DeviceCreateRequest {
                create_options: CreateOptions::FILE_DELETE_ON_CLOSE,
                ..create_request_on(
                    device_id,
                    &roots.path_through(device_id, "f.txt"),
                    CreateDisposition::FILE_OPEN,
                )
            },
        );
        assert_eq!(response_status(&opened), NtStatus::SUCCESS);
        response_file_id(&opened)
    });
    std::fs::rename(roots.outer.join("a"), roots.outer.join("z")).expect("rename locally");

    close_on(&mut backend, SECOND, inner_id);
    close_on(&mut backend, FIRST, outer_id);

    assert_eq!(names_in(&roots.outer.join("z").join("b")), Vec::<String>::new());
}

/// Once a drive has a root of its own, a file marked for deletion keeps the name each drive marked it through last, and loses the one marked last that still names it.
/// The file has the hard links `a.txt`, `b.txt` and `c.txt` beneath the inner root, which each case marks through the drives in the order given.
/// A local process then removes some of them, and the last handle closes.
// oracle: mutant a mark kept beside the earlier name of its drive, a name tried before one marked after it, more than one name removed, or a name another drive marked discarded
#[rstest]
#[case::the_name_marked_last_through_the_inner_drive(&[(FIRST, "a.txt"), (SECOND, "b.txt")], &[], &["a.txt", "c.txt"])]
#[case::the_name_marked_last_through_the_outer_drive(&[(SECOND, "b.txt"), (FIRST, "a.txt")], &[], &["b.txt", "c.txt"])]
#[case::the_name_marked_before_through_the_other_drive_once_the_last_is_gone(
    &[(FIRST, "a.txt"), (SECOND, "b.txt")],
    &["b.txt"],
    &["c.txt"]
)]
#[case::the_name_a_drive_marked_last_of_its_two(&[(SECOND, "a.txt"), (SECOND, "b.txt")], &[], &["a.txt", "c.txt"])]
#[case::nothing_once_the_name_a_drive_marked_last_of_its_two_is_gone(
    &[(SECOND, "a.txt"), (SECOND, "b.txt")],
    &["b.txt"],
    &["a.txt", "c.txt"]
)]
#[case::the_name_a_drive_marked_again_after_the_other_drive_marked(
    &[(SECOND, "a.txt"), (FIRST, "c.txt"), (SECOND, "b.txt")],
    &[],
    &["a.txt", "c.txt"]
)]
#[case::nothing_once_every_name_but_the_one_a_drive_marked_before_is_gone(
    &[(SECOND, "a.txt"), (FIRST, "c.txt"), (SECOND, "b.txt")],
    &["b.txt", "c.txt"],
    &["a.txt"]
)]
fn a_file_marked_through_nested_drives_loses_the_name_marked_last_that_still_names_it(
    #[case] marks: &[(u32, &str)],
    #[case] removed_locally: &[&str],
    #[case] left: &[&str],
) {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("a.txt"), b"linked").expect("write file");
    for link in ["b.txt", "c.txt"] {
        std::fs::hard_link(roots.inner.join("a.txt"), roots.inner.join(link)).expect("link file");
    }
    let mut backend = roots.backend();
    let handles = marks
        .iter()
        .map(|&(device_id, name)| {
            let path = roots.path_through(device_id, name);
            (
                device_id,
                create_on(&mut backend, device_id, &path, CreateDisposition::FILE_OPEN),
            )
        })
        .collect::<Vec<_>>();
    for &(device_id, file_id) in &handles {
        let marked = set_information_on(&mut backend, device_id, file_id, delete_pending(true));
        assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    }
    for name in removed_locally {
        std::fs::remove_file(roots.inner.join(name)).expect("remove locally");
    }

    for (device_id, file_id) in handles {
        close_on(&mut backend, device_id, file_id);
    }

    // Nothing else beneath either root is touched: the outer root holds only the directories that lead to the inner one.
    let expected = [roots.outer.join("a"), roots.inner.clone()]
        .into_iter()
        .chain(left.iter().map(|name| roots.inner.join(name)))
        .collect::<Vec<_>>();
    assert_eq!(entries_below(&roots.outer), expected);
}

/// Clearing the mark through either drive clears every name the file was marked through, so nothing is removed when its last handle closes.
// oracle: spec MS-FSA §2.1.5.15.3
#[rstest]
#[case::through_the_drive_that_marked_first(0)]
#[case::through_the_drive_that_marked_last(1)]
fn clearing_delete_pending_through_one_nested_drive_keeps_a_file_marked_through_both(#[case] clearing: usize) {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("kept.txt"), b"kept").expect("write file");
    let mut backend = roots.backend();
    let devices = [SECOND, FIRST];
    let handles = devices
        .into_iter()
        .zip(roots.open_through(&mut backend, devices, "kept.txt"));
    for (device_id, file_id) in handles.clone() {
        let marked = set_information_on(&mut backend, device_id, file_id, delete_pending(true));
        assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    }

    let (device_id, file_id) = handles.clone().nth(clearing).expect("a handle of each drive");
    let cleared = set_information_on(&mut backend, device_id, file_id, delete_pending(false));
    for (device_id, file_id) in handles {
        close_on(&mut backend, device_id, file_id);
    }

    assert_eq!(response_status(&cleared), NtStatus::SUCCESS);
    assert_eq!(names_in(&roots.inner), ["kept.txt"]);
}

/// With only a shared directory, a file keeps the one name it was marked for deletion through last: a mark through another drive replaces it, as a mark through the same drive does.
/// The file has the hard links `a.txt`, marked through one drive, and `b.txt`, marked through the other after it.
/// Only `b.txt` is removed, and nothing once a local process removed it.
// oracle: mutant a backend with only a shared directory kept the name an earlier mark through another drive gave the file
#[rstest]
#[case::the_name_marked_last_is_removed(&[])]
#[case::no_other_name_is_removed_once_the_name_marked_last_is_gone(&["b.txt"])]
fn a_file_marked_through_two_drives_of_the_shared_directory_loses_only_the_name_marked_last(
    #[case] removed_locally: &[&str],
) {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("a.txt"), b"linked").expect("write file");
    std::fs::hard_link(fixture.share.join("a.txt"), fixture.share.join("b.txt")).expect("link file");
    let factory = drives_of_the_shared_directory(&utf8(&fixture.share));
    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    let backend = nix_backend(backend.as_mut());
    let handles = [(FIRST, r"\a.txt"), (SECOND, r"\b.txt")].map(|(device_id, path)| {
        (
            device_id,
            create_on(backend, device_id, path, CreateDisposition::FILE_OPEN),
        )
    });
    for (device_id, file_id) in handles {
        let marked = set_information_on(backend, device_id, file_id, delete_pending(true));
        assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    }
    for name in removed_locally {
        std::fs::remove_file(fixture.share.join(name)).expect("remove locally");
    }

    for (device_id, file_id) in handles {
        close_on(backend, device_id, file_id);
    }

    assert_eq!(names_in(&fixture.share), ["a.txt"]);
}

/// The drives without a root of their own keep one marked name between them while another drive has a root of its own, because they all reach the file through the one root of the shared directory.
/// The file has the hard links `a.txt`, marked through one such drive, and `b.txt`, marked through another after it.
/// A local process removes `b.txt`, and `a.txt` is still there after the last handle closes, because the later mark replaced it.
// oracle: mutant a name marked through a drive of the shared directory kept beside the name another such drive marked after it, once a drive has a root of its own
#[test]
fn a_mark_through_a_drive_of_the_shared_directory_replaces_the_name_of_another_beside_a_drive_with_its_own_root() {
    const OWN_ROOT: u32 = 3;
    let roots = TwoRoots::new();
    std::fs::write(roots.first.join("a.txt"), b"linked").expect("write file");
    std::fs::hard_link(roots.first.join("a.txt"), roots.first.join("b.txt")).expect("link file");
    let mut backend = NixRdpdrBackend::new(utf8(&roots.first)).with_drive_root(OWN_ROOT, utf8(&roots.second));
    let handles = [(FIRST, r"\a.txt"), (SECOND, r"\b.txt")].map(|(device_id, path)| {
        (
            device_id,
            create_on(&mut backend, device_id, path, CreateDisposition::FILE_OPEN),
        )
    });
    for (device_id, file_id) in handles {
        let marked = set_information_on(&mut backend, device_id, file_id, delete_pending(true));
        assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    }
    std::fs::remove_file(roots.first.join("b.txt")).expect("remove locally");

    for (device_id, file_id) in handles {
        close_on(&mut backend, device_id, file_id);
    }

    assert_eq!(names_in(&roots.first), ["a.txt"]);
}

/// Drives with roots of their own keep a marked name each although their roots name the same directory, because each holds it open through its own path and one may remove a file that the other cannot.
/// The file has the hard links `a.txt`, marked through one drive, and `b.txt`, marked through the other after it.
/// A local process removes `b.txt`, and the file loses `a.txt` when its last handle closes.
// oracle: mutant drives with roots of their own that name the same directory kept one marked name between them
#[test]
fn a_mark_through_a_drive_with_its_own_root_keeps_the_name_of_another_with_the_same_root() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("a.txt"), b"linked").expect("write file");
    std::fs::hard_link(fixture.share.join("a.txt"), fixture.share.join("b.txt")).expect("link file");
    let factory = drives_with_the_same_root(&utf8(&fixture.share));
    let (mut backend, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
    let backend = nix_backend(backend.as_mut());
    let handles = [(FIRST, r"\a.txt"), (SECOND, r"\b.txt")].map(|(device_id, path)| {
        (
            device_id,
            create_on(backend, device_id, path, CreateDisposition::FILE_OPEN),
        )
    });
    for (device_id, file_id) in handles {
        let marked = set_information_on(backend, device_id, file_id, delete_pending(true));
        assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    }
    std::fs::remove_file(fixture.share.join("b.txt")).expect("remove locally");

    for (device_id, file_id) in handles {
        close_on(backend, device_id, file_id);
    }

    assert_eq!(names_in(&fixture.share), Vec::<String>::new());
}

/// A file keeps the four names it was marked through last, so the names kept for it do not grow with the drives that reach it.
/// Six drives with roots of their own name one directory, and each marks the file through a hard link of its own, `link-1.txt` through the first to `link-6.txt` through the last.
/// A local process then removes the links marked last, and the delete has to do with the names that are left when the handles close.
/// The fourth name from the last still removes the file, and no name before it is tried: the links marked through the two earliest drives stay in both cases.
// oracle: mutant a name kept for every root a file was marked through, or fewer than the four marked last kept
#[rstest]
#[case::the_fourth_name_from_the_last_removes_the_file(3)]
#[case::no_name_before_the_fourth_from_the_last_is_tried(4)]
fn a_file_marked_through_six_drives_keeps_the_four_names_marked_last(#[case] removed_locally: usize) {
    let fixture = Fixture::new();
    let links = [1, 2, 3, 4, 5, 6].map(|device_id| (device_id, format!("link-{device_id}.txt")));
    std::fs::write(fixture.share.join("link-1.txt"), b"linked").expect("write file");
    for (_, name) in &links[1..] {
        std::fs::hard_link(fixture.share.join("link-1.txt"), fixture.share.join(name)).expect("link file");
    }
    let mut backend = links
        .iter()
        .fold(NixRdpdrBackend::default(), |backend, (device_id, _)| {
            backend.with_drive_root(*device_id, utf8(&fixture.share))
        });
    let handles = links.each_ref().map(|(device_id, name)| {
        let path = format!(r"\{name}");
        (
            *device_id,
            create_on(&mut backend, *device_id, &path, CreateDisposition::FILE_OPEN),
        )
    });
    for (device_id, file_id) in handles {
        let marked = set_information_on(&mut backend, device_id, file_id, delete_pending(true));
        assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    }
    for (_, name) in links.iter().rev().take(removed_locally) {
        std::fs::remove_file(fixture.share.join(name)).expect("remove locally");
    }

    for (device_id, file_id) in handles {
        close_on(&mut backend, device_id, file_id);
    }

    assert_eq!(names_in(&fixture.share), ["link-1.txt", "link-2.txt"]);
}

/// A rename does not replace a file that is open through the other drive that reaches it, whichever drive is the enclosing one.
// oracle: spec MS-FSA §2.1.5.15.12
#[rstest]
fn rename_with_replace_refuses_a_target_open_through_a_drive_nested_with_the_renaming_one(
    #[values([FIRST, SECOND], [SECOND, FIRST])] devices: [u32; 2],
) {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("target.txt"), b"old").expect("write target");
    std::fs::write(roots.inner.join("source.txt"), b"new").expect("write source");
    let mut backend = roots.backend();
    let [renaming, other] = devices;
    let _target_id = create_on(
        &mut backend,
        other,
        &roots.path_through(other, "target.txt"),
        CreateDisposition::FILE_OPEN,
    );
    let source_id = create_on(
        &mut backend,
        renaming,
        &roots.path_through(renaming, "source.txt"),
        CreateDisposition::FILE_OPEN,
    );

    let response = set_information_on(
        &mut backend,
        renaming,
        source_id,
        rename_to(&roots.path_through(renaming, "target.txt"), true),
    );

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert_eq!(
        std::fs::read(roots.inner.join("target.txt")).expect("read target"),
        b"old"
    );
    assert!(roots.inner.join("source.txt").exists());
}

/// A file or directory that is open through one drive cannot be renamed through the other drive that reaches it, whichever drive is the enclosing one.
/// A path names a file only beneath the root of its drive, so the handle of the other drive could not take the new name.
/// Once the other drive closes its handle, the same rename goes ahead.
/// Each case names what is renamed, what the other drive holds open, and the new name, all beneath the inner root.
// oracle: mutant a file or directory open through a drive that serves another directory renamed, or the rename refused with another status
#[rstest]
#[case::the_file("x.txt", "x.txt", "y.txt")]
#[case::another_hard_link_of_the_file("linked.txt", "link.txt", "y.txt")]
#[case::a_directory("dir", "dir", "moved")]
// The rename is refused before the filesystem is asked what the new name refers to, so the case holds whether or not the filesystem tells the two spellings apart.
#[case::the_file_renamed_to_another_spelling_of_its_name("x.txt", "x.txt", "X.txt")]
fn a_rename_is_refused_while_a_drive_nested_with_the_renaming_one_holds_the_file_open(
    #[case] renamed: &str,
    #[case] open: &str,
    #[case] to: &str,
    #[values([FIRST, SECOND], [SECOND, FIRST])] devices: [u32; 2],
) {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("x.txt"), b"open").expect("write file");
    std::fs::write(roots.inner.join("linked.txt"), b"linked").expect("write file");
    std::fs::hard_link(roots.inner.join("linked.txt"), roots.inner.join("link.txt")).expect("link file");
    std::fs::create_dir(roots.inner.join("dir")).expect("create directory");
    let mut backend = roots.backend();
    let [renaming, other] = devices;
    let renaming_id = create_on(
        &mut backend,
        renaming,
        &roots.path_through(renaming, renamed),
        CreateDisposition::FILE_OPEN,
    );
    let other_id = create_on(
        &mut backend,
        other,
        &roots.path_through(other, open),
        CreateDisposition::FILE_OPEN,
    );
    let rename = rename_to(&roots.path_through(renaming, to), false);
    let before = entries_below(&roots.sandbox);

    let refused = set_information_on(&mut backend, renaming, renaming_id, rename.clone());
    let after_the_refusal = entries_below(&roots.sandbox);
    close_on(&mut backend, other, other_id);
    let renamed_after_the_close = set_information_on(&mut backend, renaming, renaming_id, rename);

    assert_eq!(response_status(&refused), NtStatus::SHARING_VIOLATION);
    assert_eq!(after_the_refusal, before);
    // oracle: spec MS-FSA §2.1.5.15.12
    assert_eq!(response_status(&renamed_after_the_close), NtStatus::SUCCESS);
    let names = names_in(&roots.inner);
    assert!(
        names.iter().any(|name| name == to) && !names.iter().any(|name| name == renamed),
        "{renamed} is stored as {to} among {names:?}"
    );
}

/// A handle on the root of its drive names that directory whatever it is called, so it does not keep the enclosing drive from renaming the root of the inner drive.
/// The inner drive goes on serving the renamed directory.
/// A file that the inner drive holds open inside its root still keeps the root from being renamed, as it does for any directory holding open files.
// oracle: mutant a handle on the root of its drive counted among the handles that refuse a rename through a drive that serves another directory, or the inner drive served nothing after its root was renamed
#[test]
fn the_enclosing_drive_renames_the_root_of_the_inner_drive_while_the_inner_drive_holds_only_that_root_open() {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("open.txt"), b"open").expect("write file");
    let mut backend = roots.backend();
    let _root_id = create_on(&mut backend, SECOND, r"\", CreateDisposition::FILE_OPEN);
    let file_id = create_on(&mut backend, SECOND, r"\open.txt", CreateDisposition::FILE_OPEN);
    let directory_id = create_on(&mut backend, FIRST, r"\a\b", CreateDisposition::FILE_OPEN);
    let rename = rename_to(r"\a\c", false);

    let refused = set_information_on(&mut backend, FIRST, directory_id, rename.clone());
    let names_after_the_refusal = names_in(&roots.outer.join("a"));
    close_on(&mut backend, SECOND, file_id);
    let renamed = set_information_on(&mut backend, FIRST, directory_id, rename);
    let created = create_with(
        &mut backend,
        create_request_on(SECOND, r"\new.txt", CreateDisposition::FILE_CREATE),
    );

    // oracle: spec MS-FSA §2.1.5.15.12
    assert_eq!(response_status(&refused), NtStatus::ACCESS_DENIED);
    assert_eq!(names_after_the_refusal, ["b"]);
    assert_eq!(response_status(&renamed), NtStatus::SUCCESS);
    assert_eq!(response_status(&created), NtStatus::SUCCESS);
    assert_eq!(names_in(&roots.outer.join("a")), ["c"]);
    assert_eq!(names_in(&roots.outer.join("a").join("c")), ["new.txt", "open.txt"]);
}

/// With three drives nested in one another, the outermost cannot move a file that the middle one holds open, to a name outside the root of the middle one as to any other.
// oracle: mutant a file open through the middle of three nested drives moved through the outermost, or the rename refused with another status
#[test]
fn the_outermost_of_three_nested_drives_cannot_move_a_file_open_through_the_middle_one() {
    const MIDDLE: u32 = 3;
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("f.txt"), b"open").expect("write file");
    let mut backend = roots.backend().with_drive_root(MIDDLE, utf8(&roots.outer.join("a")));
    let outer_id = create_on(&mut backend, FIRST, r"\a\b\f.txt", CreateDisposition::FILE_OPEN);
    let _middle_id = create_on(&mut backend, MIDDLE, r"\b\f.txt", CreateDisposition::FILE_OPEN);
    let before = entries_below(&roots.sandbox);

    let response = set_information_on(&mut backend, FIRST, outer_id, rename_to(r"\out.txt", false));

    assert_eq!(response_status(&response), NtStatus::SHARING_VIOLATION);
    assert_eq!(entries_below(&roots.sandbox), before);
}

/// A handle without DELETE access is refused for that, whatever else would refuse the rename, such as a handle of the other drive on the same file.
// oracle: spec MS-FSA §2.1.5.15.12
#[test]
fn a_rename_through_a_handle_without_delete_access_is_refused_for_that_while_another_drive_holds_the_file_open() {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("x.txt"), b"open").expect("write file");
    let mut backend = roots.backend();
    let _inner_id = create_on(&mut backend, SECOND, r"\x.txt", CreateDisposition::FILE_OPEN);
    let opened = create_with(
        &mut backend,
        DeviceCreateRequest {
            desired_access: DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY,
            ..create_request_on(FIRST, r"\a\b\x.txt", CreateDisposition::FILE_OPEN)
        },
    );

    let response = set_information_on(
        &mut backend,
        FIRST,
        response_file_id(&opened),
        rename_to(r"\a\b\y.txt", false),
    );

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
}

/// A rename of a file that is open through a drive that serves another directory is refused for that, before anything else about the file or its new name is checked.
/// Each case would otherwise be refused with another status: STATUS_ACCESS_DENIED for a file marked for deletion, STATUS_OBJECT_NAME_INVALID for a name Windows cannot hold.
// oracle: mutant a rename of a file open through a drive that serves another directory refused for another reason first
#[rstest]
#[case::a_file_marked_for_deletion(
    |backend: &mut NixRdpdrBackend, file_id: u32| {
        let marked = set_information_on(backend, SECOND, file_id, delete_pending(true));
        assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    },
    r"\a\b\y.txt"
)]
#[case::a_new_name_windows_cannot_hold(|_: &mut NixRdpdrBackend, _: u32| {}, r"\a\b\y*.txt")]
fn a_rename_is_refused_for_a_handle_of_another_drive_before_any_other_check(
    #[case] prepare: fn(&mut NixRdpdrBackend, u32),
    #[case] to: &str,
) {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("x.txt"), b"open").expect("write file");
    let mut backend = roots.backend();
    let [outer_id, inner_id] = roots.open_through(&mut backend, [FIRST, SECOND], "x.txt");
    prepare(&mut backend, inner_id);

    let response = set_information_on(&mut backend, FIRST, outer_id, rename_to(to, false));

    assert_eq!(response_status(&response), NtStatus::SHARING_VIOLATION);
}

/// The enclosing drive cannot rename the root of the inner drive, or a directory below it, while the inner drive holds a file open inside that directory.
// oracle: spec MS-FSA §2.1.5.15.12
#[rstest]
#[case::the_root_of_the_inner_drive(r"\a\b")]
#[case::a_directory_below_it(r"\a\b\sub")]
fn the_enclosing_drive_cannot_rename_a_directory_holding_files_open_through_the_inner_drive(#[case] directory: &str) {
    let roots = DeepRoots::new();
    std::fs::create_dir(roots.inner.join("sub")).expect("create directory");
    std::fs::write(roots.inner.join("sub").join("open.txt"), b"open").expect("write file");
    let mut backend = roots.backend();
    let _file_id = create_on(&mut backend, SECOND, r"\sub\open.txt", CreateDisposition::FILE_OPEN);
    let directory_id = create_on(&mut backend, FIRST, directory, CreateDisposition::FILE_OPEN);

    let response = set_information_on(&mut backend, FIRST, directory_id, rename_to(r"\moved", false));

    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert!(roots.inner.join("sub").join("open.txt").exists());
}

/// The enclosing drive can rename a directory above the root of the inner drive while the inner drive holds a file open.
/// A drive tracks the directories from its own root down, so the enclosing drive does not see that file below the directory it renames.
/// The inner drive then serves the same directory at its new path.
// oracle: mutant a drive loses its directory when an enclosing drive renames a directory above it
#[test]
fn a_drive_follows_its_directory_when_the_enclosing_drive_renames_a_directory_above_it() {
    let roots = DeepRoots::new();
    std::fs::write(roots.inner.join("open.txt"), b"open").expect("write file");
    let mut backend = roots.backend();
    let file_id = create_on(&mut backend, SECOND, r"\open.txt", CreateDisposition::FILE_OPEN);
    let directory_id = create_on(&mut backend, FIRST, r"\a", CreateDisposition::FILE_OPEN);

    let renamed = set_information_on(&mut backend, FIRST, directory_id, rename_to(r"\moved", false));
    let read = read_on(&mut backend, SECOND, file_id, 0, 16);
    let _ = create_with(
        &mut backend,
        create_request_on(SECOND, r"\new.txt", CreateDisposition::FILE_CREATE),
    );
    let reopened = create_on(&mut backend, SECOND, r"\open.txt", CreateDisposition::FILE_OPEN);
    let read_again = read_on(&mut backend, SECOND, reopened, 0, 16);

    assert_eq!(response_status(&renamed), NtStatus::SUCCESS);
    assert_eq!(response_read_data(&read), b"open");
    assert_eq!(response_read_data(&read_again), b"open");
    assert_eq!(names_in(&roots.outer.join("moved").join("b")), ["new.txt", "open.txt"]);
    assert_eq!(names_in(&roots.outer), ["moved"]);
}

/// A handle on the root of the inner drive names that root wherever it lies, so it takes no path from a rename of the root through the enclosing drive.
/// The enclosing drive can therefore still rename the directory above the inner root, as it can before it renames the root.
// oracle: mutant a handle on the root of a drive took the directories that lead to that root from the root of the enclosing drive
#[test]
fn a_directory_above_the_inner_root_can_be_renamed_after_the_root_was_renamed_while_the_inner_drive_holds_it_open() {
    let roots = DeepRoots::new();
    let mut backend = roots.backend();
    let _root_id = create_on(&mut backend, SECOND, r"\", CreateDisposition::FILE_OPEN);
    rename_on(&mut backend, FIRST, r"\a\b", r"\a\c");
    let directory_id = create_on(&mut backend, FIRST, r"\a", CreateDisposition::FILE_OPEN);

    let response = set_information_on(&mut backend, FIRST, directory_id, rename_to(r"\moved", false));

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(names_in(&roots.outer), ["moved"]);
}

/// The enclosing drive can delete the root of the inner drive once it is empty and no file in it is open.
/// The inner drive then has no directory to create in, and its creates make nothing anywhere else.
// oracle: mutant a drive whose directory was deleted served the directory now at its path, or another one
#[test]
fn a_drive_whose_directory_the_enclosing_drive_deleted_creates_nothing() {
    let roots = DeepRoots::new();
    let mut backend = roots.backend();
    let directory_id = create_on(&mut backend, FIRST, r"\a\b", CreateDisposition::FILE_OPEN);
    let marked = set_information_on(&mut backend, FIRST, directory_id, delete_pending(true));
    close_on(&mut backend, FIRST, directory_id);
    let is_deleted = !roots.inner.exists();
    // A directory now at the path of the deleted one is not the directory the drive was configured with.
    std::fs::create_dir(&roots.inner).expect("create directory");
    let before = entries_below(&roots.sandbox);

    let file = create_with(
        &mut backend,
        create_request_on(SECOND, r"\new.txt", CreateDisposition::FILE_CREATE),
    );
    let directory = create_with(
        &mut backend,
        DeviceCreateRequest {
            create_options: CreateOptions::FILE_DIRECTORY_FILE,
            ..create_request_on(SECOND, r"\new", CreateDisposition::FILE_CREATE)
        },
    );

    // oracle: spec MS-FSA §2.1.5.15.3
    assert_eq!(response_status(&marked), NtStatus::SUCCESS);
    assert!(is_deleted);
    assert_ne!(response_status(&file), NtStatus::SUCCESS);
    assert_ne!(response_status(&directory), NtStatus::SUCCESS);
    assert_eq!(entries_below(&roots.sandbox), before);
}

/// Devices `FIRST` and `SECOND` announced over the one shared directory.
fn drives_of_the_shared_directory(share: &str) -> NixRdpdrBackendFactory {
    NixRdpdrBackendFactory::new(share.to_owned())
        .with_drive(FIRST, "first".to_owned())
        .with_drive(SECOND, "second".to_owned())
}

/// Devices `FIRST` and `SECOND` each given `share` as its own root.
fn drives_with_the_same_root(share: &str) -> NixRdpdrBackendFactory {
    NixRdpdrBackendFactory::default()
        .with_drive_root(FIRST, "first".to_owned(), share.to_owned())
        .with_drive_root(SECOND, "second".to_owned(), share.to_owned())
}

/// Devices `FIRST` and `SECOND` each given `share` as its own root, `SECOND` through a symbolic link beside `share`, which lies in no root.
fn drives_with_the_same_root_under_two_spellings(share: &str) -> NixRdpdrBackendFactory {
    let alias = format!("{share}-alias");
    std::os::unix::fs::symlink(share, &alias).expect("link directory");
    NixRdpdrBackendFactory::default()
        .with_drive_root(FIRST, "first".to_owned(), share.to_owned())
        .with_drive_root(SECOND, "second".to_owned(), alias)
}

/// A backend whose devices `FIRST` and `SECOND` serve `roots`, given their roots in `order`.
fn backend_with_roots_in_order(roots: &[PathBuf; 2], order: [u32; 2]) -> NixRdpdrBackend {
    order
        .into_iter()
        .fold(NixRdpdrBackend::default(), |backend, device_id| {
            let root = if device_id == FIRST { &roots[0] } else { &roots[1] };
            backend.with_drive_root(device_id, utf8(root))
        })
}

/// Renames `from` to `to` through `device_id`, with a handle opened for the rename and closed after it.
fn rename_on(backend: &mut NixRdpdrBackend, device_id: u32, from: &str, to: &str) {
    let file_id = create_on(backend, device_id, from, CreateDisposition::FILE_OPEN);
    let response = set_information_on(backend, device_id, file_id, rename_to(to, false));
    assert_eq!(response_status(&response), NtStatus::SUCCESS, "rename {from} to {to}");
    close_on(backend, device_id, file_id);
}

/// Asks to rename `from` to `to` through `device_id` as [`rename_on`] does, whatever the answers, and returns the status of the open the rename needs.
fn try_rename_on(backend: &mut NixRdpdrBackend, device_id: u32, from: &str, to: &str) -> NtStatus {
    let opened = create_with(
        backend,
        create_request_on(device_id, from, CreateDisposition::FILE_OPEN),
    );
    let file_id = response_file_id(&opened);
    let _ = set_information_on(backend, device_id, file_id, rename_to(to, false));
    let _ = close_on(backend, device_id, file_id);
    response_status(&opened)
}

/// Makes `link` a symbolic link to `target`.
fn link(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("create symbolic link");
}

/// The path of every entry below `directory`, sorted, without following symbolic links.
fn entries_below(directory: &Path) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    let mut pending = vec![directory.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("list directory") {
            let entry = entry.expect("read entry");
            if entry.file_type().expect("entry type").is_dir() {
                pending.push(entry.path());
            }
            entries.push(entry.path());
        }
    }
    entries.sort();
    entries
}

fn nix_backend(backend: &mut dyn RdpdrBackend) -> &mut NixRdpdrBackend {
    backend
        .as_any_mut()
        .downcast_mut()
        .expect("the factory builds a NixRdpdrBackend")
}

/// A share holding a 16-byte file opened through two handles.
fn two_handles() -> (Fixture, NixRdpdrBackend, u32, u32) {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("data.bin"), [0; 16]).expect("write file");
    let mut backend = fixture.backend();
    let first_id = create(&mut backend, r"\data.bin", CreateDisposition::FILE_OPEN);
    let second_id = create(&mut backend, r"\data.bin", CreateDisposition::FILE_OPEN);
    (fixture, backend, first_id, second_id)
}

fn lock(
    backend: &mut NixRdpdrBackend,
    file_id: u32,
    completion_id: u32,
    operation: LockOperation,
    wait: bool,
    offset: u64,
    length: u64,
) -> Vec<SvcMessage> {
    send(
        backend,
        lock_request(
            DeviceIoRequest {
                completion_id,
                ..request_header(file_id, MajorFunction::LockControl)
            },
            operation,
            wait,
            offset,
            length,
        ),
    )
}

fn lock_request(
    device_io_request: DeviceIoRequest,
    operation: LockOperation,
    wait: bool,
    offset: u64,
    length: u64,
) -> ServerDriveIoRequest {
    ServerDriveIoRequest::ServerDriveLockControlRequest(ServerDriveLockControlRequest {
        device_io_request,
        operation,
        wait,
        locks: vec![RdpLockInfo { length, offset }],
    })
}

fn read(backend: &mut NixRdpdrBackend, file_id: u32, offset: u64, length: u32) -> Vec<SvcMessage> {
    read_on(backend, FIRST, file_id, offset, length)
}

fn read_on(backend: &mut NixRdpdrBackend, device_id: u32, file_id: u32, offset: u64, length: u32) -> Vec<SvcMessage> {
    send(
        backend,
        ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
            device_io_request: header_on(device_id, file_id, MajorFunction::Read),
            length,
            offset,
        }),
    )
}

fn write(backend: &mut NixRdpdrBackend, file_id: u32, offset: u64, data: &[u8]) -> Vec<SvcMessage> {
    write_on(backend, FIRST, file_id, offset, data)
}

fn write_on(backend: &mut NixRdpdrBackend, device_id: u32, file_id: u32, offset: u64, data: &[u8]) -> Vec<SvcMessage> {
    send(
        backend,
        ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: header_on(device_id, file_id, MajorFunction::Write),
            offset,
            write_data: data.to_vec(),
        }),
    )
}

fn send(backend: &mut NixRdpdrBackend, request: ServerDriveIoRequest) -> Vec<SvcMessage> {
    backend
        .handle_drive_io_request(request)
        .expect("complete drive request")
}

/// The completion ID and status of each response, ordered by completion ID, because nothing orders independent completions.
fn completions(messages: &[SvcMessage]) -> Vec<(u32, NtStatus)> {
    let mut completions = messages
        .iter()
        .map(|message| (completion_id(message), message_status(message)))
        .collect::<Vec<_>>();
    completions.sort_by_key(|(completion_id, _)| *completion_id);
    completions
}

fn completion_id(message: &SvcMessage) -> u32 {
    read_u32(std::slice::from_ref(message), COMPLETION_ID)
}

fn message_status(message: &SvcMessage) -> NtStatus {
    response_status(std::slice::from_ref(message))
}

fn rename_to(file_name: &str, replace_if_exists: bool) -> FileInformationClass {
    FileInformationClass::Rename(FileRenameInformation {
        replace_if_exists: if replace_if_exists {
            Boolean::True
        } else {
            Boolean::False
        },
        file_name: file_name.to_owned(),
    })
}

fn delete_pending(delete_pending: bool) -> FileInformationClass {
    FileInformationClass::Disposition(FileDispositionInformation {
        delete_pending: u8::from(delete_pending),
    })
}

/// Queries `pattern` and then continues until enumeration ends, returning the names in order and the final status.
fn list_directory(backend: &mut NixRdpdrBackend, file_id: u32, pattern: &str) -> (Vec<String>, NtStatus) {
    list_directory_on(backend, FIRST, file_id, pattern)
}

fn list_directory_on(
    backend: &mut NixRdpdrBackend,
    device_id: u32,
    file_id: u32,
    pattern: &str,
) -> (Vec<String>, NtStatus) {
    let mut names = Vec::new();
    let mut response = query_directory_on(backend, device_id, file_id, pattern, true);
    while response_status(&response) == NtStatus::SUCCESS {
        names.push(response_file_name(&response));
        response = query_directory_on(backend, device_id, file_id, "", false);
    }
    (names, response_status(&response))
}

fn query_standard_information(backend: &mut NixRdpdrBackend, file_id: u32) -> FileStandardInformation {
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryInformationRequest(
            ServerDriveQueryInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::QueryInformation),
                file_info_class_lvl: FileInformationClassLevel::FILE_STANDARD_INFORMATION,
            },
        ))
        .expect("complete query information");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let response = response[0]
        .encode_unframed_pdu()
        .expect("encode query information response");
    let buffer = &response[INFORMATION_BUFFER..];
    let information = FileInformationClass::decode(
        FileInformationClassLevel::FILE_STANDARD_INFORMATION,
        buffer.len(),
        &mut ReadCursor::new(buffer),
    )
    .expect("decode FileStandardInformation");
    let FileInformationClass::Standard(information) = information else {
        panic!("expected FileStandardInformation, got {information:?}");
    };
    information
}

fn create_with(backend: &mut NixRdpdrBackend, request: DeviceCreateRequest) -> Vec<SvcMessage> {
    backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
        .expect("complete create")
}

fn set_information(backend: &mut NixRdpdrBackend, file_id: u32, set_buffer: FileInformationClass) -> Vec<SvcMessage> {
    set_information_on(backend, FIRST, file_id, set_buffer)
}

fn set_information_on(
    backend: &mut NixRdpdrBackend,
    device_id: u32,
    file_id: u32,
    set_buffer: FileInformationClass,
) -> Vec<SvcMessage> {
    send(backend, set_information_request(device_id, file_id, set_buffer))
}

fn set_information_request(device_id: u32, file_id: u32, set_buffer: FileInformationClass) -> ServerDriveIoRequest {
    ServerDriveIoRequest::ServerDriveSetInformationRequest(ServerDriveSetInformationRequest {
        device_io_request: header_on(device_id, file_id, MajorFunction::SetInformation),
        set_buffer,
    })
}

fn create(backend: &mut NixRdpdrBackend, path: &str, disposition: CreateDisposition) -> u32 {
    create_on(backend, FIRST, path, disposition)
}

fn create_on(backend: &mut NixRdpdrBackend, device_id: u32, path: &str, disposition: CreateDisposition) -> u32 {
    let response = create_with(backend, create_request_on(device_id, path, disposition));
    assert_eq!(
        response_status(&response),
        NtStatus::SUCCESS,
        "create {path} through device {device_id}"
    );
    response_file_id(&response)
}

fn close(backend: &mut NixRdpdrBackend, file_id: u32) -> Vec<SvcMessage> {
    close_on(backend, FIRST, file_id)
}

fn close_on(backend: &mut NixRdpdrBackend, device_id: u32, file_id: u32) -> Vec<SvcMessage> {
    send(
        backend,
        ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: header_on(device_id, file_id, MajorFunction::Close),
        }),
    )
}

fn query_directory(backend: &mut NixRdpdrBackend, file_id: u32, path: &str, initial_query: bool) -> Vec<SvcMessage> {
    query_directory_on(backend, FIRST, file_id, path, initial_query)
}

fn query_directory_on(
    backend: &mut NixRdpdrBackend,
    device_id: u32,
    file_id: u32,
    path: &str,
    initial_query: bool,
) -> Vec<SvcMessage> {
    send(
        backend,
        query_directory_request(device_id, file_id, path, initial_query),
    )
}

fn query_directory_request(device_id: u32, file_id: u32, path: &str, initial_query: bool) -> ServerDriveIoRequest {
    ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(ServerDriveQueryDirectoryRequest {
        device_io_request: header_on(device_id, file_id, MajorFunction::DirectoryControl),
        file_info_class_lvl: FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION,
        initial_query: u8::from(initial_query),
        path: path.to_owned(),
    })
}

fn create_request(path: &str, disposition: CreateDisposition) -> DeviceCreateRequest {
    create_request_on(FIRST, path, disposition)
}

fn create_request_on(device_id: u32, path: &str, disposition: CreateDisposition) -> DeviceCreateRequest {
    DeviceCreateRequest {
        device_io_request: header_on(device_id, 0, MajorFunction::Create),
        // FILE_READ_DATA, FILE_WRITE_DATA, FILE_READ_ATTRIBUTES, DELETE and SYNCHRONIZE, so a test can rename and delete through any handle.
        desired_access: DesiredAccess::from_bits_retain(0x0011_0083),
        allocation_size: 0,
        file_attributes: FileAttributes::FILE_ATTRIBUTE_NORMAL,
        shared_access: SharedAccess::from_bits_retain(0x0000_0007),
        create_disposition: disposition,
        create_options: CreateOptions::empty(),
        path: path.to_owned(),
    }
}

fn request_header(file_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
    header_on(FIRST, file_id, major_function)
}

fn header_on(device_id: u32, file_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
    DeviceIoRequest {
        device_id,
        file_id,
        completion_id: 7,
        major_function,
        minor_function: MinorFunction::from(0),
    }
}

// Every response starts with an RDPDR_HEADER, Component and PacketId, followed by DeviceId, CompletionId and IoStatus,
// all four bytes ([MS-RDPEFS] 2.2.1.1 Shared Header (RDPDR_HEADER) and 2.2.1.5 Device I/O Response (DR_DEVICE_IOCOMPLETION)).
const COMPLETION_ID: Range<usize> = 8..12;
const IO_STATUS: Range<usize> = 12..16;
// [MS-RDPEFS] 2.2.1.5.1 Device Create Response (DR_CREATE_RSP) continues with FileId.
const CREATE_FILE_ID: Range<usize> = 16..20;
// [MS-RDPEFS] 2.2.1.5.3 Device Read Response (DR_READ_RSP) continues with Length and the data read.
const READ_DATA: usize = 20;
// [MS-RDPEFS] 2.2.3.4.10 Client Drive Query Directory Response (DR_DRIVE_QUERY_DIRECTORY_RSP) continues with Length
// and the entry it describes.
const DIRECTORY_ENTRY: usize = 20;
// [MS-RDPEFS] 2.2.3.4.8 Client Drive Query Information Response (DR_DRIVE_QUERY_INFORMATION_RSP) continues with Length
// and the information it describes.
const INFORMATION_BUFFER: usize = 20;

fn response_status(messages: &[SvcMessage]) -> NtStatus {
    NtStatus::from(read_u32(messages, IO_STATUS))
}

fn response_file_id(messages: &[SvcMessage]) -> u32 {
    read_u32(messages, CREATE_FILE_ID)
}

fn response_file_name(messages: &[SvcMessage]) -> String {
    let response = messages[0]
        .encode_unframed_pdu()
        .expect("encode query directory response");
    let entry = &response[DIRECTORY_ENTRY..];
    let entry = FileInformationClass::decode(
        FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION,
        entry.len(),
        &mut ReadCursor::new(entry),
    )
    .expect("decode directory entry");
    let FileInformationClass::BothDirectory(entry) = entry else {
        panic!("expected a FileBothDirectoryInformation entry, got {entry:?}");
    };
    entry.file_name
}

/// The bytes of each response, to compare whole responses.
fn encoded(messages: &[SvcMessage]) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| message.encode_unframed_pdu().expect("encode RDPDR response"))
        .collect()
}

/// The data of a Device Read Response.
fn response_read_data(messages: &[SvcMessage]) -> Vec<u8> {
    let response = messages[0].encode_unframed_pdu().expect("encode read response");
    response[READ_DATA..].to_vec()
}

fn read_u32(messages: &[SvcMessage], field: Range<usize>) -> u32 {
    let response = messages[0].encode_unframed_pdu().expect("encode RDPDR response");
    u32::from_le_bytes(response[field].try_into().expect("four-byte field"))
}

/// The status of a create through a drive that is served nothing, which a create beneath a shared directory that cannot be opened fails with.
const NOTHING_SERVED: NtStatus = NtStatus::UNSUCCESSFUL;

/// The devices the tests address: every helper without a device parameter addresses `FIRST`.
const FIRST: u32 = 1;
const SECOND: u32 = 2;

/// Two directories in one sandbox, each the root of one drive.
struct TwoRoots {
    first: PathBuf,
    second: PathBuf,
    /// Removes both directories when dropped.
    _fixture: Fixture,
}

impl TwoRoots {
    fn new() -> Self {
        let fixture = Fixture::new();
        let second = fixture.sandbox.join("second");
        std::fs::create_dir(&second).expect("create second root");
        Self {
            first: fixture.share.clone(),
            second,
            _fixture: fixture,
        }
    }

    /// A backend without a shared directory, whose devices `FIRST` and `SECOND` serve the two roots.
    fn backend(&self) -> NixRdpdrBackend {
        NixRdpdrBackend::default()
            .with_drive_root(FIRST, utf8(&self.first))
            .with_drive_root(SECOND, utf8(&self.second))
    }
}

/// A directory and its `sub` directory, each the root of one drive, so both drives reach the files of the inner one.
struct NestedRoots {
    inner: PathBuf,
    fixture: Fixture,
}

impl NestedRoots {
    fn new() -> Self {
        let fixture = Fixture::new();
        let inner = fixture.share.join("sub");
        std::fs::create_dir(&inner).expect("create inner root");
        Self { inner, fixture }
    }
}

/// An outer root holding an inner root two levels down, at `a/b`.
/// Beside `a` lies `d`, whose entry `b` is a symbolic link to `outside`, a directory outside both roots.
struct SwappedRoot {
    outer: PathBuf,
    outside: PathBuf,
    _fixture: Fixture,
}

impl SwappedRoot {
    fn new() -> Self {
        let fixture = Fixture::new();
        let outer = fixture.share.clone();
        let outside = fixture.sandbox.join("outside");
        std::fs::create_dir_all(outer.join("a").join("b")).expect("create inner root");
        std::fs::write(outer.join("a").join("b").join("inner.txt"), b"inner").expect("write file");
        std::fs::create_dir(&outside).expect("create outside directory");
        std::fs::write(outside.join("outside.txt"), b"outside").expect("write file");
        std::fs::create_dir(outer.join("d")).expect("create directory");
        std::os::unix::fs::symlink(&outside, outer.join("d").join("b")).expect("link directory");
        Self {
            outer,
            outside,
            _fixture: fixture,
        }
    }

    /// Announces `FIRST` over the outer root and `SECOND` over the inner one.
    fn factory(&self) -> NixRdpdrBackendFactory {
        NixRdpdrBackendFactory::default()
            .with_drive_root(FIRST, "outer".to_owned(), utf8(&self.outer))
            .with_drive_root(SECOND, "inner".to_owned(), utf8(&self.outer.join("a").join("b")))
    }

    fn backend(&self) -> NixRdpdrBackend {
        NixRdpdrBackend::default()
            .with_drive_root(FIRST, utf8(&self.outer))
            .with_drive_root(SECOND, utf8(&self.outer.join("a").join("b")))
    }

    /// Renames `a` to `a_old` and `d` to `a` through the outer drive, so the path of the inner root leads through the link.
    fn swap_through_the_outer_drive(&self, backend: &mut NixRdpdrBackend) {
        rename_on(backend, FIRST, r"\a", r"\a_old");
        rename_on(backend, FIRST, r"\d", r"\a");
    }
}

/// A directory and the directory `a/b` two levels below it, each the root of one drive, so both drives reach the files of the inner one.
struct DeepRoots {
    sandbox: PathBuf,
    outer: PathBuf,
    inner: PathBuf,
    _fixture: Fixture,
}

impl DeepRoots {
    fn new() -> Self {
        let fixture = Fixture::new();
        let inner = fixture.share.join("a").join("b");
        std::fs::create_dir_all(&inner).expect("create inner root");
        Self {
            sandbox: fixture.sandbox.clone(),
            outer: fixture.share.clone(),
            inner,
            _fixture: fixture,
        }
    }

    /// A backend without a shared directory, whose device `FIRST` serves the outer root and `SECOND` the inner one.
    fn backend(&self) -> NixRdpdrBackend {
        NixRdpdrBackend::default()
            .with_drive_root(FIRST, utf8(&self.outer))
            .with_drive_root(SECOND, utf8(&self.inner))
    }

    /// The path that names `name`, a path beneath the inner root, through `device_id`.
    fn path_through(&self, device_id: u32, name: &str) -> String {
        if device_id == FIRST {
            format!(r"\a\b\{name}")
        } else {
            format!(r"\{name}")
        }
    }

    /// Opens `name`, a path beneath the inner root, through each of `devices`, and returns their handles in that order.
    fn open_through(&self, backend: &mut NixRdpdrBackend, devices: [u32; 2], name: &str) -> [u32; 2] {
        devices.map(|device_id| {
            let path = self.path_through(device_id, name);
            create_on(backend, device_id, &path, CreateDisposition::FILE_OPEN)
        })
    }
}

fn utf8(path: &Path) -> String {
    path.to_str().expect("UTF-8 temporary directory").to_owned()
}

/// The names in `directory`, sorted.
fn names_in(directory: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(directory)
        .expect("list directory")
        .map(|entry| {
            entry
                .expect("read entry")
                .file_name()
                .into_string()
                .expect("UTF-8 name")
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// A temporary directory holding the shared directory, so escapes land next to it.
struct Fixture {
    sandbox: PathBuf,
    share: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT_FIXTURE: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "rdpdr-native-nix-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        );
        let sandbox = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_dir_all(&sandbox);
        let share = sandbox.join("share");
        std::fs::create_dir_all(&share).expect("create fixture share");
        Self { sandbox, share }
    }

    fn backend(&self) -> NixRdpdrBackend {
        NixRdpdrBackend::new(utf8(&self.share))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.sandbox);
    }
}
