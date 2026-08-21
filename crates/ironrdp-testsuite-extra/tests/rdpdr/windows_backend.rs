#![cfg(windows)]

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ironrdp_core as _;
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    AnyIoCtlCode, CreateDisposition, CreateOptions, DecodedDeviceControlRequest, DesiredAccess, DeviceCloseRequest,
    DeviceControlRequest, DeviceCreateRequest, DeviceFlushBuffersRequest, DeviceIoRequest, DeviceReadRequest,
    DeviceWriteRequest, FileAttributes, FileEndOfFileInformation, FileInformationClass, FileInformationClassLevel,
    FileSystemInformationClassLevel, LockOperation, MajorFunction, MinorFunction, NtStatus, RdpLockInfo,
    SecurityInformation, ServerDriveIoRequest, ServerDriveLockControlRequest, ServerDriveNotifyChangeDirectoryRequest,
    ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest, ServerDriveQuerySecurityRequest,
    ServerDriveQueryVolumeInformationRequest, ServerDriveSetInformationRequest, ServerDriveSetSecurityRequest,
    SharedAccess,
};
use ironrdp_rdpdr_native::{RedirectedDrive, WindowsRdpdrBackend, WindowsRdpdrBackendFactory};
use ironrdp_svc::SvcMessage;
use tracing as _;

const MAX_STATIC_IO_SIZE: usize = 1_024 * 1_024;

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
fn synchronous_nonalert_create_option_is_accepted() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    activate(&mut backend);

    let mut create = create_request(
        &fixture.relative(r"root\synchronous.txt"),
        CreateDisposition::FILE_OPEN_IF,
    );
    create.create_options = CreateOptions::FILE_SYNCHRONOUS_IO_NONALERT;
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
        .expect("complete synchronous create");

    assert_eq!(response_status(&response), NtStatus::SUCCESS);
}

#[test]
fn read_only_drives_allow_reads_and_reject_mutations() {
    let fixture = Fixture::new();
    let file_path = fixture.root.join("root").join("read-only.txt");
    std::fs::write(&file_path, b"safe").expect("write read-only fixture");

    let mut backend = fixture.read_only_backend();
    activate(&mut backend);

    let mut open = create_request(&fixture.relative(r"root\read-only.txt"), CreateDisposition::FILE_OPEN);
    open.desired_access = DesiredAccess::from_bits_retain(0x0010_0081);
    let open_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(open))
        .expect("complete read-only open");
    assert_eq!(response_status(&open_response), NtStatus::SUCCESS);
    let file_id = response_file_id(&open_response);

    let read_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
            device_io_request: request_header(file_id, MajorFunction::Read),
            length: 4,
            offset: 0,
        }))
        .expect("complete read-only read");
    assert_eq!(response_status(&read_response), NtStatus::SUCCESS);
    let bytes = read_response[0].encode_unframed_pdu().expect("encode read response");
    assert_eq!(&bytes[20..], b"safe");

    let access_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
            &fixture.relative(r"root\read-only.txt"),
            CreateDisposition::FILE_OPEN,
        )))
        .expect("complete write-access open");
    assert_eq!(response_status(&access_response), NtStatus::MEDIA_WRITE_PROTECTED);

    let mut create = create_request(&fixture.relative(r"root\new.txt"), CreateDisposition::FILE_OPEN_IF);
    create.desired_access = DesiredAccess::from_bits_retain(0x0010_0081);
    let create_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
        .expect("complete create on read-only drive");
    assert_eq!(response_status(&create_response), NtStatus::MEDIA_WRITE_PROTECTED);
    assert!(!fixture.root.join("root").join("new.txt").exists());

    let write_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"overwrite".to_vec(),
        }))
        .expect("complete write on read-only drive");
    assert_eq!(response_status(&write_response), NtStatus::MEDIA_WRITE_PROTECTED);

    let set_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 0 }),
            },
        ))
        .expect("complete metadata update on read-only drive");
    assert_eq!(response_status(&set_response), NtStatus::MEDIA_WRITE_PROTECTED);
    assert_eq!(
        std::fs::read(file_path).expect("read fixture after rejected mutations"),
        b"safe"
    );
}

#[test]
fn static_io_limit_is_inclusive() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    activate(&mut backend);

    let create_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
            &fixture.relative(r"root\bounded.bin"),
            CreateDisposition::FILE_OPEN_IF,
        )))
        .expect("complete bounded create");
    assert_eq!(response_status(&create_response), NtStatus::SUCCESS);
    let file_id = response_file_id(&create_response);

    let write_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: vec![0xA5; MAX_STATIC_IO_SIZE],
        }))
        .expect("complete maximum write");
    assert_eq!(response_status(&write_response), NtStatus::SUCCESS);

    let read_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
            device_io_request: request_header(file_id, MajorFunction::Read),
            length: u32::try_from(MAX_STATIC_IO_SIZE).expect("static I/O size fits in u32"),
            offset: 0,
        }))
        .expect("complete maximum read");
    assert_eq!(response_status(&read_response), NtStatus::SUCCESS);

    let oversized_write = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: vec![0; MAX_STATIC_IO_SIZE + 1],
        }))
        .expect("complete oversized write");
    assert_eq!(response_status(&oversized_write), NtStatus::INVALID_PARAMETER);

    let oversized_read = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
            device_io_request: request_header(file_id, MajorFunction::Read),
            length: u32::try_from(MAX_STATIC_IO_SIZE + 1).expect("oversized I/O size fits in u32"),
            offset: 0,
        }))
        .expect("complete oversized read");
    assert_eq!(response_status(&oversized_read), NtStatus::INVALID_PARAMETER);
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

#[test]
fn advanced_filesystem_operations_are_handle_bound() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    activate(&mut backend);

    let file_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
            &fixture.relative(r"root\advanced.txt"),
            CreateDisposition::FILE_OPEN_IF,
        )))
        .expect("complete advanced file create");
    assert_eq!(response_status(&file_response), NtStatus::SUCCESS);
    let file_id = response_file_id(&file_response);

    let stream_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryInformationRequest(
            ServerDriveQueryInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::QueryInformation),
                file_info_class_lvl: FileInformationClassLevel::FILE_STREAM_INFORMATION,
            },
        ))
        .expect("complete stream query");
    assert_eq!(response_status(&stream_response), NtStatus::SUCCESS);

    let flush_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceFlushBuffersRequest(
            DeviceFlushBuffersRequest {
                device_io_request: request_header(file_id, MajorFunction::FlushBuffers),
            },
        ))
        .expect("complete explicit flush");
    assert_eq!(response_status(&flush_response), NtStatus::SUCCESS);

    let lock_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveLockControlRequest(
            ServerDriveLockControlRequest {
                device_io_request: request_header(file_id, MajorFunction::LockControl),
                operation: LockOperation::Exclusive,
                wait: false,
                locks: vec![RdpLockInfo { length: 4, offset: 0 }],
            },
        ))
        .expect("complete byte-range lock");
    assert_eq!(response_status(&lock_response), NtStatus::SUCCESS);

    let unlock_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveLockControlRequest(
            ServerDriveLockControlRequest {
                device_io_request: request_header(file_id, MajorFunction::LockControl),
                operation: LockOperation::Unlock,
                wait: false,
                locks: vec![RdpLockInfo { length: 4, offset: 0 }],
            },
        ))
        .expect("complete byte-range unlock");
    assert_eq!(response_status(&unlock_response), NtStatus::SUCCESS);

    let control_response = backend
        .handle_drive_device_control(DecodedDeviceControlRequest {
            request: DeviceControlRequest {
                header: request_header(file_id, MajorFunction::DeviceControl),
                output_buffer_length: 2,
                input_buffer_length: 0,
                io_control_code: AnyIoCtlCode(0x0009_003c),
            },
            input_buffer: Vec::new(),
        })
        .expect("complete compression query");
    assert_eq!(response_status(&control_response), NtStatus::SUCCESS);

    let directory_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(
            directory_create_request(&fixture.relative("root")),
        ))
        .expect("open directory");
    assert_eq!(response_status(&directory_response), NtStatus::SUCCESS);
    let directory_id = response_file_id(&directory_response);

    let query_directory_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(
            ServerDriveQueryDirectoryRequest {
                device_io_request: request_header(directory_id, MajorFunction::DirectoryControl),
                file_info_class_lvl: FileInformationClassLevel::FILE_NAMES_INFORMATION,
                initial_query: 1,
                path: fixture.relative(r"root\*"),
            },
        ))
        .expect("complete directory query");
    assert_eq!(response_status(&query_directory_response), NtStatus::SUCCESS);

    let volume_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(
            ServerDriveQueryVolumeInformationRequest {
                device_io_request: request_header(directory_id, MajorFunction::QueryVolumeInformation),
                fs_info_class_lvl: FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION,
            },
        ))
        .expect("complete volume query");
    assert_eq!(response_status(&volume_response), NtStatus::SUCCESS);
}

#[test]
fn directory_notifications_are_cancelled_by_close() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    activate(&mut backend);

    let directory_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(
            directory_create_request(&fixture.relative("root")),
        ))
        .expect("open watched directory");
    assert_eq!(response_status(&directory_response), NtStatus::SUCCESS);
    let directory_id = response_file_id(&directory_response);

    let scheduled = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(
            ServerDriveNotifyChangeDirectoryRequest {
                device_io_request: request_header(directory_id, MajorFunction::DirectoryControl),
                watch_tree: 0,
                completion_filter: 1,
            },
        ))
        .expect("schedule directory notification");
    assert!(scheduled.is_empty());

    let close_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(directory_id, MajorFunction::Close),
        }))
        .expect("close watched directory");
    assert_eq!(close_response.len(), 2);
    assert_eq!(response_status(&close_response[..1]), NtStatus::SUCCESS);
    assert_eq!(response_status(&close_response[1..]), NtStatus::SUCCESS);
    assert!(
        RdpdrBackend::poll_deferred_messages(&mut backend)
            .expect("poll cancelled notification")
            .is_empty()
    );
}

#[test]
fn security_descriptor_query_and_set_use_the_opened_handle() {
    let fixture = Fixture::new();
    let mut backend = fixture.backend();
    activate(&mut backend);

    let mut create = create_request(&fixture.relative(r"root\security.txt"), CreateDisposition::FILE_OPEN_IF);
    create.desired_access = DesiredAccess::from_bits_retain(0x0016_0083);
    let create_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
        .expect("create security test file");
    assert_eq!(response_status(&create_response), NtStatus::SUCCESS);
    let file_id = response_file_id(&create_response);

    let query_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQuerySecurityRequest(
            ServerDriveQuerySecurityRequest {
                device_io_request: request_header(file_id, MajorFunction::QuerySecurity),
                security_information: SecurityInformation::OWNER
                    | SecurityInformation::GROUP
                    | SecurityInformation::DACL,
            },
        ))
        .expect("query security descriptor");
    assert_eq!(response_status(&query_response), NtStatus::SUCCESS);
    let bytes = query_response[0]
        .encode_unframed_pdu()
        .expect("encode security response");
    let descriptor_length = usize::try_from(u32::from_le_bytes(
        bytes[16..20].try_into().expect("security descriptor length"),
    ))
    .expect("descriptor length fits in usize");
    let descriptor = bytes[20..20 + descriptor_length].to_vec();
    assert!(!descriptor.is_empty());

    let set_response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetSecurityRequest(
            ServerDriveSetSecurityRequest {
                device_io_request: request_header(file_id, MajorFunction::SetSecurity),
                security_information: SecurityInformation::DACL,
                security_descriptor: descriptor,
            },
        ))
        .expect("set security descriptor");
    assert_eq!(response_status(&set_response), NtStatus::SUCCESS);
}

fn activate(backend: &mut WindowsRdpdrBackend) {
    RdpdrBackend::restore_drive(backend, 1).expect("activate test drive");
}

fn directory_create_request(path: &str) -> DeviceCreateRequest {
    DeviceCreateRequest {
        device_io_request: request_header(0, MajorFunction::Create),
        desired_access: DesiredAccess::from_bits_retain(0x0010_0001),
        allocation_size: 0,
        file_attributes: FileAttributes::empty(),
        shared_access: SharedAccess::from_bits_retain(0x0000_0007),
        create_disposition: CreateDisposition::FILE_OPEN,
        create_options: CreateOptions::from_bits_retain(0x0000_0021),
        path: path.to_owned(),
    }
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
        self.backend_with_read_only(false)
    }

    fn read_only_backend(&self) -> WindowsRdpdrBackend {
        self.backend_with_read_only(true)
    }

    fn backend_with_read_only(&self, read_only: bool) -> WindowsRdpdrBackend {
        WindowsRdpdrBackendFactory::new(
            RedirectedDrive::new(1, "Test", &self.volume_root, read_only).expect("valid test drive"),
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
