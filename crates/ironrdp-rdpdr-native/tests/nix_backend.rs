#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use ironrdp_core::ReadCursor;
use ironrdp_pdu as _;
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCreateRequest, DeviceIoRequest,
    DeviceReadRequest, DeviceWriteRequest, FileAttributes, FileDispositionInformation, FileInformationClass,
    FileInformationClassLevel, FileRenameInformation, FileStandardInformation, MajorFunction, MinorFunction, NtStatus,
    ServerDriveIoRequest, ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest,
    ServerDriveSetInformationRequest, SharedAccess,
};
use ironrdp_rdpdr_native::backend::NixRdpdrBackend;
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
    let mut names = Vec::new();
    let mut response = query_directory(backend, file_id, pattern, true);
    while response_status(&response) == NtStatus::SUCCESS {
        names.push(response_file_name(&response));
        response = query_directory(backend, file_id, "", false);
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
    backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer,
            },
        ))
        .expect("complete set information")
}

fn create(backend: &mut NixRdpdrBackend, path: &str, disposition: CreateDisposition) -> u32 {
    let response = create_with(backend, create_request(path, disposition));
    assert_eq!(response_status(&response), NtStatus::SUCCESS, "create {path}");
    response_file_id(&response)
}

fn close(backend: &mut NixRdpdrBackend, file_id: u32) {
    backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .expect("complete close");
}

fn query_directory(backend: &mut NixRdpdrBackend, file_id: u32, path: &str, initial_query: bool) -> Vec<SvcMessage> {
    backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(
            ServerDriveQueryDirectoryRequest {
                device_io_request: request_header(file_id, MajorFunction::DirectoryControl),
                file_info_class_lvl: FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION,
                initial_query: u8::from(initial_query),
                path: path.to_owned(),
            },
        ))
        .expect("complete query directory")
}

fn create_request(path: &str, disposition: CreateDisposition) -> DeviceCreateRequest {
    DeviceCreateRequest {
        device_io_request: request_header(0, MajorFunction::Create),
        desired_access: DesiredAccess::from_bits_retain(0x0010_0083),
        allocation_size: 0,
        file_attributes: FileAttributes::FILE_ATTRIBUTE_NORMAL,
        shared_access: SharedAccess::from_bits_retain(0x0000_0007),
        create_disposition: disposition,
        create_options: CreateOptions::empty(),
        path: path.to_owned(),
    }
}

fn request_header(file_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
    DeviceIoRequest {
        device_id: 1,
        file_id,
        completion_id: 7,
        major_function,
        minor_function: MinorFunction::from(0),
    }
}

// Every response starts with an RDPDR_HEADER, Component and PacketId, followed by DeviceId, CompletionId and IoStatus,
// all four bytes ([MS-RDPEFS] 2.2.1.1 Shared Header (RDPDR_HEADER) and 2.2.1.5 Device I/O Response (DR_DEVICE_IOCOMPLETION)).
const IO_STATUS: Range<usize> = 12..16;
// [MS-RDPEFS] 2.2.1.5.1 Device Create Response (DR_CREATE_RSP) continues with FileId.
const CREATE_FILE_ID: Range<usize> = 16..20;
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

fn read_u32(messages: &[SvcMessage], field: Range<usize>) -> u32 {
    let response = messages[0].encode_unframed_pdu().expect("encode RDPDR response");
    u32::from_le_bytes(response[field].try_into().expect("four-byte field"))
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
        NixRdpdrBackend::new(self.share.to_str().expect("UTF-8 temporary directory").to_owned())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.sandbox);
    }
}
