#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use ironrdp_core as _;
use ironrdp_pdu as _;
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCreateRequest, DeviceIoRequest,
    DeviceReadRequest, DeviceWriteRequest, FileAttributes, FileDispositionInformation, FileInformationClass,
    FileInformationClassLevel, FileRenameInformation, MajorFunction, MinorFunction, NtStatus, ServerDriveIoRequest,
    ServerDriveQueryDirectoryRequest, ServerDriveSetInformationRequest, SharedAccess,
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

    // oracle: spec MS-FSA §2.1.5.15.12
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
#[case::existing_file_with_create(
    r"\file.txt",
    CreateOptions::empty(),
    CreateDisposition::FILE_CREATE,
    NtStatus::OBJECT_NAME_COLLISION
)]
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

/// Each operation that resolves a server-supplied path works on paths inside the share.
#[test]
fn file_lifecycle_inside_the_share() {
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

    std::fs::write(fixture.share.join("docs").join("a.txt"), b"a").expect("write a.txt");
    let docs_id = create(&mut backend, r"\docs", CreateDisposition::FILE_OPEN);
    // The directory returns `.`, `..`, `new` and `a.txt`, and enumeration then ends.
    // oracle: spec MS-FSA §2.1.5.6.3
    assert_eq!(
        list_directory(&mut backend, docs_id, r"\docs\*"),
        [
            NtStatus::SUCCESS,
            NtStatus::SUCCESS,
            NtStatus::SUCCESS,
            NtStatus::SUCCESS,
            NtStatus::NO_MORE_FILES
        ]
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
        [NtStatus::SUCCESS, NtStatus::NO_MORE_FILES]
    );
}

// oracle: spec MS-FSA §2.1.5.6.3
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

// oracle: spec MS-FSA §2.1.5.1.2
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

// oracle: spec MS-FSA §2.1.5.1
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

// oracle: spec MS-FSA §2.1.5.5
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

// oracle: spec MS-FSA §2.1.5.1
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

    assert_eq!(response_status(&reopen), NtStatus::DELETE_PENDING);
    assert!(still_there);
    assert!(!fixture.share.join("shared.txt").exists());
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

/// Returns the status of an initial query for `pattern` followed by queries until enumeration ends.
fn list_directory(backend: &mut NixRdpdrBackend, file_id: u32, pattern: &str) -> Vec<NtStatus> {
    let mut statuses = vec![response_status(&query_directory(backend, file_id, pattern, true))];
    while statuses.last() == Some(&NtStatus::SUCCESS) {
        statuses.push(response_status(&query_directory(backend, file_id, "", false)));
    }
    statuses
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

fn response_status(messages: &[SvcMessage]) -> NtStatus {
    let response = messages[0].encode_unframed_pdu().expect("encode RDPDR response");
    NtStatus::from(u32::from_le_bytes(
        response[12..16].try_into().expect("response status"),
    ))
}

fn response_file_id(messages: &[SvcMessage]) -> u32 {
    let response = messages[0].encode_unframed_pdu().expect("encode create response");
    u32::from_le_bytes(response[16..20].try_into().expect("create file ID"))
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
