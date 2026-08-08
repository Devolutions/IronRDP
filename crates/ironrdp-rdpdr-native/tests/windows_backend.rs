#![cfg(windows)]

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ironrdp_core as _;
use ironrdp_pdu as _;
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCreateRequest, DeviceIoRequest,
    DeviceReadRequest, DeviceWriteRequest, FileAttributes, FileEndOfFileInformation, FileInformationClass,
    FileInformationClassLevel, MajorFunction, MinorFunction, NtStatus, ServerDriveIoRequest,
    ServerDriveQueryInformationRequest, ServerDriveSetInformationRequest, SharedAccess,
};
use ironrdp_rdpdr_native::{RedirectedDrive, WindowsRdpdrBackend, WindowsRdpdrBackendFactory};
use ironrdp_svc::SvcMessage;
use windows as _;

#[test]
fn filesystem_lifecycle_is_handle_relative_and_bounded() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    activate(&mut backend);

    let create = create_request(&fixture.relative(r"root\report.txt"), CreateDisposition::FILE_OPEN_IF);
    let create_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
        .expect("complete create");
    assert_eq!(response_status(&create_response), NtStatus::SUCCESS);
    let file_id = response_file_id(&create_response);

    let write = DeviceWriteRequest {
        device_io_request: request_header(file_id, MajorFunction::Write),
        offset: 0,
        write_data: b"hello".to_vec(),
    };
    let write_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(write))
        .expect("complete write");
    assert_eq!(response_status(&write_response), NtStatus::SUCCESS);

    let append_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: u64::MAX,
            write_data: b"!".to_vec(),
        }))
        .expect("complete append");
    assert_eq!(response_status(&append_response), NtStatus::SUCCESS);

    let read_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
            device_io_request: request_header(file_id, MajorFunction::Read),
            length: 6,
            offset: 0,
        }))
        .expect("complete read");
    assert_eq!(response_status(&read_response), NtStatus::SUCCESS);
    let bytes = read_response[0].encode_unframed_pdu().expect("encode read response");
    assert_eq!(&bytes[20..], b"hello!");

    for file_info_class_lvl in [
        FileInformationClassLevel::FILE_BASIC_INFORMATION,
        FileInformationClassLevel::FILE_STANDARD_INFORMATION,
        FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION,
    ] {
        let query_response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryInformationRequest(
                ServerDriveQueryInformationRequest {
                    device_io_request: request_header(file_id, MajorFunction::QueryInformation),
                    file_info_class_lvl,
                },
            ))
            .expect("complete metadata query");
        assert_eq!(response_status(&query_response), NtStatus::SUCCESS);
    }

    let set_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 5 }),
            },
        ))
        .expect("complete set information");
    assert_eq!(response_status(&set_response), NtStatus::SUCCESS);
    assert_eq!(
        std::fs::read(fixture.root.join("root").join("report.txt")).expect("read test file"),
        b"hello"
    );

    let close_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .expect("complete close");
    assert_eq!(response_status(&close_response), NtStatus::SUCCESS);
}

#[test]
fn hostile_paths_never_open_outside_the_selected_volume() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    activate(&mut backend);

    for path in [
        r"\..\outside.txt",
        r"\??\C:\outside.txt",
        r"\root\CON",
        r"\root\file:stream",
    ] {
        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
                path,
                CreateDisposition::FILE_OPEN_IF,
            )))
            .expect("complete hostile create");
        assert_ne!(response_status(&response), NtStatus::SUCCESS);
    }
}

#[test]
fn reparse_points_cannot_escape_the_trusted_volume_root() {
    let fixture = Fixture::new();
    let outside = fixture.sandbox.join("outside");
    std::fs::create_dir_all(&outside).expect("create outside directory");
    std::fs::write(outside.join("sentinel.txt"), b"outside").expect("write sentinel");

    let link = fixture.root.join("root").join("escape");
    if let Err(error) = std::os::windows::fs::symlink_dir(&outside, &link) {
        if error.raw_os_error() == Some(1314) {
            return;
        }
        panic!("create test reparse point: {error}");
    }

    let mut backend = fixture.backend();
    activate(&mut backend);
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
            &fixture.relative(r"root\escape\sentinel.txt"),
            CreateDisposition::FILE_OPEN,
        )))
        .expect("complete reparse create");

    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(
        std::fs::read(outside.join("sentinel.txt")).expect("read sentinel"),
        b"outside"
    );
}

fn activate(backend: &mut WindowsRdpdrBackend) {
    RdpdrBackend::restore_drive(backend, 1).expect("activate test drive");
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

struct Fixture {
    sandbox: PathBuf,
    root: PathBuf,
    volume_root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let workspace = std::env::current_dir().expect("current workspace directory");
        let volume_root = workspace
            .ancestors()
            .last()
            .expect("workspace has a volume root")
            .to_owned();
        let unique = format!(
            "rdpdr-native-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after Unix epoch")
                .as_nanos()
        );
        let sandbox = workspace.join("target").join(unique);
        let root = sandbox.join("root");
        std::fs::create_dir_all(root.join("root")).expect("create fixture root");
        Self {
            sandbox,
            root,
            volume_root,
        }
    }

    fn backend(&self) -> WindowsRdpdrBackend {
        WindowsRdpdrBackendFactory::new(
            RedirectedDrive::new(1, "Test", &self.volume_root, false).expect("valid test drive"),
        )
        .build()
    }

    fn relative(&self, suffix: &str) -> String {
        let relative = self
            .root
            .strip_prefix(&self.volume_root)
            .expect("fixture is beneath its volume root");
        format!(r"\{}\{}", relative.display(), suffix).replace('/', r"\")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.sandbox);
    }
}
