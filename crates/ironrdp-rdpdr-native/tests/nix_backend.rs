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
use nix as _;
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

    let request = DeviceCreateRequest {
        create_options,
        ..create_request(path, CreateDisposition::FILE_CREATE)
    };
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
        .expect("complete create");

    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    assert!(!fixture.sandbox.join("escape.txt").exists());
}

// oracle: reference crates/ironrdp-rdpdr-native/tests/windows_backend.rs
#[test]
fn symbolic_links_in_the_share_are_not_followed() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.sandbox.join("outside")).expect("create outside directory");
    let sentinel = fixture.sandbox.join("outside").join("sentinel.txt");
    std::fs::write(&sentinel, b"outside").expect("write sentinel");
    std::os::unix::fs::symlink(fixture.sandbox.join("outside"), fixture.share.join("dir_link"))
        .expect("link directory");
    std::os::unix::fs::symlink(&sentinel, fixture.share.join("file_link")).expect("link file");
    let mut backend = fixture.backend();

    for path in [r"\dir_link\sentinel.txt", r"\file_link"] {
        for disposition in [CreateDisposition::FILE_OPEN, CreateDisposition::FILE_OVERWRITE_IF] {
            let response = backend
                .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
                    path,
                    disposition,
                )))
                .expect("complete create");
            assert_ne!(response_status(&response), NtStatus::SUCCESS, "{path} {disposition:?}");
        }
    }
    assert_eq!(std::fs::read(&sentinel).expect("read sentinel"), b"outside");
}

// oracle: reference crates/ironrdp-rdpdr-native/src/windows/path.rs
#[test]
fn rename_cannot_move_a_file_outside_the_share() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\inside.txt", CreateDisposition::FILE_CREATE);

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::Rename(FileRenameInformation {
                    replace_if_exists: Boolean::False,
                    file_name: r"\..\moved.txt".to_owned(),
                }),
            },
        ))
        .expect("complete rename");

    assert_ne!(response_status(&response), NtStatus::SUCCESS);
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

    assert_ne!(response_status(&response), NtStatus::SUCCESS);
}

#[test]
fn reads_longer_than_the_limit_are_rejected() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("data.bin"), vec![7; 16]).expect("write data");
    let mut backend = fixture.backend();
    let file_id = create(&mut backend, r"\data.bin", CreateDisposition::FILE_OPEN);

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/file.rs
    for (length, status) in [
        (MAX_READ_LENGTH, NtStatus::SUCCESS),
        (MAX_READ_LENGTH + 1, NtStatus::INVALID_PARAMETER),
        (u32::MAX, NtStatus::INVALID_PARAMETER),
    ] {
        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
                device_io_request: request_header(file_id, MajorFunction::Read),
                length,
                offset: 0,
            }))
            .expect("complete read");
        assert_eq!(response_status(&response), status, "read of {length} bytes");
    }
}

/// Each operation that resolves a server-supplied path works on paths inside the share.
#[test]
fn file_lifecycle_inside_the_share() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.share.join("docs")).expect("create subdirectory");
    let mut backend = fixture.backend();

    let directory_id = create_directory(&mut backend, r"\docs\new");
    assert!(fixture.share.join("docs").join("new").is_dir());
    close(&mut backend, directory_id);

    let file_id = create(&mut backend, r"\docs\report.txt", CreateDisposition::FILE_CREATE);
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"hello".to_vec(),
        }))
        .expect("complete write");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::Rename(FileRenameInformation {
                    replace_if_exists: Boolean::False,
                    file_name: r"\docs\new\renamed.txt".to_owned(),
                }),
            },
        ))
        .expect("complete rename");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(
        std::fs::read(fixture.share.join("docs").join("new").join("renamed.txt")).expect("read renamed file"),
        b"hello"
    );

    // The handle follows the rename, so deleting it removes the renamed file.
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 }),
            },
        ))
        .expect("complete delete");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(!fixture.share.join("docs").join("new").join("renamed.txt").exists());
    close(&mut backend, file_id);

    std::fs::write(fixture.share.join("docs").join("a.txt"), b"a").expect("write a.txt");
    let docs_id = create(&mut backend, r"\docs", CreateDisposition::FILE_OPEN);
    // The directory holds `new` and `a.txt`, and enumeration then ends.
    // oracle: spec MS-FSA §2.1.5.6.3
    let mut statuses = vec![response_status(&query_directory(
        &mut backend,
        docs_id,
        r"\docs\*",
        true,
    ))];
    for _ in 0..2 {
        statuses.push(response_status(&query_directory(&mut backend, docs_id, "", false)));
    }
    assert_eq!(
        statuses,
        [NtStatus::SUCCESS, NtStatus::SUCCESS, NtStatus::NO_MORE_FILES]
    );
}

fn create(backend: &mut NixRdpdrBackend, path: &str, disposition: CreateDisposition) -> u32 {
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
            path,
            disposition,
        )))
        .expect("complete create");
    assert_eq!(response_status(&response), NtStatus::SUCCESS, "create {path}");
    response_file_id(&response)
}

fn create_directory(backend: &mut NixRdpdrBackend, path: &str) -> u32 {
    let mut request = create_request(path, CreateDisposition::FILE_CREATE);
    request.create_options = CreateOptions::FILE_DIRECTORY_FILE;
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
        .expect("complete create");
    assert_eq!(response_status(&response), NtStatus::SUCCESS, "create directory {path}");
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
