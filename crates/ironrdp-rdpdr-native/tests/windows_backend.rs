#![cfg(windows)]

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ironrdp_core as _;
use ironrdp_pdu as _;
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    AnyIoCtlCode, Boolean, CreateDisposition, CreateOptions, DecodedDeviceControlRequest, DesiredAccess,
    DeviceCloseRequest, DeviceControlRequest, DeviceCreateRequest, DeviceFlushBuffersRequest, DeviceIoRequest,
    DeviceReadRequest, DeviceWriteRequest, FileAttributes, FileDispositionInformation, FileEndOfFileInformation,
    FileInformationClass, FileInformationClassLevel, FileRenameInformation, FileSystemInformationClassLevel,
    LockOperation, MajorFunction, MinorFunction, NtStatus, RdpLockInfo, SecurityInformation, ServerDriveIoRequest,
    ServerDriveLockControlRequest, ServerDriveNotifyChangeDirectoryRequest, ServerDriveQueryDirectoryRequest,
    ServerDriveQueryInformationRequest, ServerDriveQuerySecurityRequest, ServerDriveQueryVolumeInformationRequest,
    ServerDriveSetInformationRequest, ServerDriveSetSecurityRequest, SharedAccess,
};
use ironrdp_rdpdr_native::{
    RedirectedDrive, WindowsDownloadSinkFactory, WindowsRdpdrBackend, WindowsRdpdrBackendFactory,
};
use ironrdp_svc::SvcMessage;
use tracing as _;
use windows as _;

const MAX_STATIC_IO_SIZE: usize = 1_024 * 1_024;

// oracle: mutant permitting a server to read client data through a download-only device.
#[test]
fn download_sink_denies_reads() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");

    let response = backend.handle_drive_io_request(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
        device_io_request: request_header(1, MajorFunction::Read),
        length: 1,
        offset: 0,
    }));

    assert!(response.is_ok(), "read request must complete with a refusal");
    assert_eq!(
        response_status(&response.expect("completed read")),
        NtStatus::ACCESS_DENIED
    );
}

// oracle: mutant refusing an exclusive download create or opening a pre-existing file.
#[test]
fn download_sink_creates_only_new_files() {
    let fixture = Fixture::new();
    let existing = fixture.root.join("existing.txt");
    std::fs::write(&existing, b"private").expect("write protected fixture");
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");

    let mut create = create_request(r"\new.part", CreateDisposition::FILE_CREATE);
    create.desired_access =
        DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE | DesiredAccess::DELETE | DesiredAccess::SYNCHRONIZE;
    let response = backend.handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create.clone()));
    assert!(response.is_ok(), "exclusive create must receive a response");
    let response = response.expect("completed create");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    // oracle: spec MS-RDPEFS §2.2.1.5.1 assigns FILE_SUPERSEDED to a successful FILE_CREATE.
    let bytes = response[0].encode_unframed_pdu().expect("encode create response");
    assert_eq!(bytes[20], 0);
    assert!(fixture.root.join("new.part").is_file());

    create.path = r"\existing.txt".to_owned();
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
        .expect("complete collision");
    assert_eq!(response_status(&response), NtStatus::OBJECT_NAME_COLLISION);
    assert_eq!(std::fs::read(existing).expect("read protected fixture"), b"private");
}

// oracle: mutant dropping writes or ignoring the server-specified file offset.
#[test]
fn download_sink_writes_at_the_requested_offset() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 2,
            write_data: b"abc".to_vec(),
        }))
        .expect("complete write");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let bytes = response[0].encode_unframed_pdu().expect("encode write response");
    assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().expect("write length")), 3);
    assert_eq!(
        std::fs::metadata(fixture.root.join("new.part"))
            .expect("file metadata")
            .len(),
        5
    );
}

// oracle: spec MS-RDPEFS §2.2.1.4 (each request targets a specific device).
#[test]
fn download_sink_rejects_file_ids_for_another_device() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");
    let mut write_request = request_header(file_id, MajorFunction::Write);
    write_request.device_id = 2;
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: write_request,
            offset: 0,
            write_data: b"wrong device".to_vec(),
        }))
        .expect("complete wrong-device write");
    assert_ne!(response_status(&response), NtStatus::SUCCESS);

    let mut close_request = request_header(file_id, MajorFunction::Close);
    close_request.device_id = 2;
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: close_request,
        }))
        .expect("complete wrong-device close");
    assert_ne!(response_status(&response), NtStatus::SUCCESS);

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"first device".to_vec(),
        }))
        .expect("complete valid write");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(
        std::fs::metadata(fixture.root.join("new.part"))
            .expect("file metadata")
            .len(),
        12
    );
}

// oracle: mutant retaining an incomplete transfer after the server closes its handle.
#[test]
fn download_sink_removes_incomplete_file_on_close() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .expect("complete close");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(!fixture.root.join("new.part").exists());
}

// oracle: mutant discarding bytes or removing a successfully published download on close.
#[test]
fn download_sink_publishes_written_bytes() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");
    let write = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"exact download bytes".to_vec(),
        }))
        .expect("complete write");
    assert_eq!(response_status(&write), NtStatus::SUCCESS);

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::Rename(FileRenameInformation {
                    replace_if_exists: Boolean::False,
                    file_name: r"\finished.txt".to_owned(),
                }),
            },
        ))
        .expect("complete publication");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let close = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .expect("complete close");
    assert_eq!(response_status(&close), NtStatus::SUCCESS);
    assert!(!fixture.root.join("new.part").exists());
    assert_eq!(
        std::fs::read(fixture.root.join("finished.txt")).expect("read published download"),
        b"exact download bytes"
    );
}

// oracle: mutant refusing to flush a newly created download.
#[test]
fn download_sink_flushes_created_file() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceFlushBuffersRequest(
            DeviceFlushBuffersRequest {
                device_io_request: request_header(file_id, MajorFunction::FlushBuffers),
            },
        ))
        .expect("complete flush");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
}

// oracle: mutant retaining an unfinished download and its file ID across channel reset.
#[test]
fn download_sink_reset_cleans_and_invalidates_open_files() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");

    backend.reset().expect("reset download channel");
    assert!(!fixture.root.join("new.part").exists());
    backend.restore_drive(1).expect("restore download device");
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"stale".to_vec(),
        }))
        .expect("complete stale write");
    assert_eq!(response_status(&response), NtStatus::INVALID_HANDLE);
}

// oracle: mutant allowing an oversized filesystem message to be reassembled.
#[test]
fn download_sink_bounds_filesystem_message_before_reassembly() {
    let fixture = Fixture::new();
    let backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();

    assert_eq!(backend.max_filesystem_message_size(), Some(1_048_632));
}

// oracle: mutant exposing unrelated client filesystem or device operations through the download sink.
#[test]
fn download_sink_denies_unrelated_device_operations() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let requests = [
        (
            "directory query",
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(ServerDriveQueryDirectoryRequest {
                device_io_request: request_header(0, MajorFunction::DirectoryControl),
                file_info_class_lvl: FileInformationClassLevel::FILE_NAMES_INFORMATION,
                initial_query: 1,
                path: r"\*".to_owned(),
            }),
        ),
        (
            "directory notification",
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(ServerDriveNotifyChangeDirectoryRequest {
                device_io_request: request_header(0, MajorFunction::DirectoryControl),
                watch_tree: 0,
                completion_filter: 1,
            }),
        ),
        (
            "volume query",
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(ServerDriveQueryVolumeInformationRequest {
                device_io_request: request_header(0, MajorFunction::QueryVolumeInformation),
                fs_info_class_lvl: FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION,
            }),
        ),
        (
            "device control",
            ServerDriveIoRequest::DeviceControlRequest(DeviceControlRequest {
                header: request_header(0, MajorFunction::DeviceControl),
                output_buffer_length: 2,
                input_buffer_length: 0,
                io_control_code: AnyIoCtlCode(0x0009_003c),
            }),
        ),
        (
            "file lock",
            ServerDriveIoRequest::ServerDriveLockControlRequest(ServerDriveLockControlRequest {
                device_io_request: request_header(0, MajorFunction::LockControl),
                operation: LockOperation::Exclusive,
                wait: false,
                locks: vec![RdpLockInfo { length: 1, offset: 0 }],
            }),
        ),
        (
            "security query",
            ServerDriveIoRequest::ServerDriveQuerySecurityRequest(ServerDriveQuerySecurityRequest {
                device_io_request: request_header(0, MajorFunction::QuerySecurity),
                security_information: SecurityInformation::OWNER,
            }),
        ),
        (
            "security update",
            ServerDriveIoRequest::ServerDriveSetSecurityRequest(ServerDriveSetSecurityRequest {
                device_io_request: request_header(0, MajorFunction::SetSecurity),
                security_information: SecurityInformation::DACL,
                security_descriptor: Vec::new(),
            }),
        ),
    ];

    for (name, request) in requests {
        let response = backend
            .handle_drive_io_request(request)
            .unwrap_or_else(|error| panic!("{name} must receive a typed denial: {error}"));
        assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED, "{name}");
    }
}

// oracle: spec MS-FSCC §2.4.47 describes the standard-information fields of the created file.
#[test]
fn download_sink_reports_created_file_metadata() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");
    let write = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"abc".to_vec(),
        }))
        .expect("complete write");
    assert_eq!(response_status(&write), NtStatus::SUCCESS);

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryInformationRequest(
            ServerDriveQueryInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::QueryInformation),
                file_info_class_lvl: FileInformationClassLevel::FILE_STANDARD_INFORMATION,
            },
        ))
        .expect("complete file query");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let bytes = response[0].encode_unframed_pdu().expect("encode information response");
    assert_eq!(i64::from_le_bytes(bytes[28..36].try_into().expect("end of file")), 3);
    assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().expect("link count")), 1);
    assert_eq!(bytes[41], 0);
}

// oracle: mutant replacing an existing client file during publication.
#[test]
fn download_sink_never_replaces_existing_files() {
    let fixture = Fixture::new();
    let existing = fixture.root.join("protected.bin");
    std::fs::write(&existing, b"private").expect("write protected fixture");
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");

    for (replace_if_exists, expected) in [
        (Boolean::True, NtStatus::ACCESS_DENIED),
        (Boolean::False, NtStatus::OBJECT_NAME_COLLISION),
    ] {
        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
                ServerDriveSetInformationRequest {
                    device_io_request: request_header(file_id, MajorFunction::SetInformation),
                    set_buffer: FileInformationClass::Rename(FileRenameInformation {
                        replace_if_exists,
                        file_name: r"\PROTECTED.bin".to_owned(),
                    }),
                },
            ))
            .expect("complete publication attempt");
        assert_eq!(response_status(&response), expected);
    }
    assert_eq!(std::fs::read(existing).expect("read protected fixture"), b"private");
}

// oracle: mutant re-resolving the approved directory path after the factory pins it.
#[test]
fn download_sink_keeps_the_approved_directory_pinned() {
    let fixture = Fixture::new();
    let factory = WindowsDownloadSinkFactory::new(&fixture.root).expect("pin download directory");
    let moved = fixture.sandbox.join("moved-root");
    std::fs::rename(&fixture.root, &moved).expect("move pinned directory");
    std::fs::create_dir(&fixture.root).expect("create replacement directory");
    let mut backend = factory.build();
    backend.restore_drive(1).expect("activate download device");

    create_download(&mut backend, r"\new.part");
    assert!(moved.join("new.part").is_file());
    assert!(!fixture.root.join("new.part").exists());
}

// oracle: mutant interpreting an untrusted name as a traversal, stream, DOS device, or nested path.
#[test]
fn download_sink_rejects_non_flat_windows_names() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");

    for path in [
        r"\..\outside.bin",
        r"\nested\child.bin",
        r"\existing.bin:stream",
        r"\\server\share",
        r"\NUL.txt",
        r"\name. ",
    ] {
        let mut request = create_request(path, CreateDisposition::FILE_CREATE);
        request.desired_access =
            DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE | DesiredAccess::DELETE | DesiredAccess::SYNCHRONIZE;
        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
            .expect("complete unsafe create");
        assert_ne!(response_status(&response), NtStatus::SUCCESS, "{path}");
    }
    assert!(!fixture.sandbox.join("outside.bin").exists());
}

// oracle: mutant permitting writes beyond the one-MiB request or one-GiB file boundary.
#[test]
fn download_sink_enforces_write_boundaries() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");

    for (offset, data, expected) in [
        (0, vec![0x5a; 1_048_576], NtStatus::SUCCESS),
        (0, vec![0x5a; 1_048_577], NtStatus::INVALID_PARAMETER),
        (1_073_741_824, Vec::new(), NtStatus::SUCCESS),
        (1_073_741_824, vec![1], NtStatus::INVALID_PARAMETER),
        (u64::MAX, vec![1], NtStatus::INVALID_PARAMETER),
    ] {
        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
                device_io_request: request_header(file_id, MajorFunction::Write),
                offset,
                write_data: data,
            }))
            .expect("complete bounded write");
        assert_eq!(response_status(&response), expected, "offset {offset}");
    }
    assert_eq!(
        std::fs::metadata(fixture.root.join("new.part"))
            .expect("file metadata")
            .len(),
        1_048_576
    );
}

// oracle: mutant using its internal cleanup permission to publish without server-requested DELETE access.
#[test]
fn download_sink_requires_requested_delete_access_to_publish() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let mut create = create_request(r"\new.part", CreateDisposition::FILE_CREATE);
    create.desired_access = DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE;
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
        .expect("complete create");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let file_id = response_file_id(&response);

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::Rename(FileRenameInformation {
                    replace_if_exists: Boolean::False,
                    file_name: r"\finished.txt".to_owned(),
                }),
            },
        ))
        .expect("complete publication attempt");
    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    assert!(!fixture.root.join("finished.txt").exists());
}

// oracle: mutant retaining an unfinished transfer after its connection is dropped.
#[test]
fn download_sink_drop_cleans_incomplete_files() {
    let fixture = Fixture::new();
    let factory = WindowsDownloadSinkFactory::new(&fixture.root).expect("pin download directory");
    {
        let mut backend = factory.build();
        backend.restore_drive(1).expect("activate download device");
        create_download(&mut backend, r"\new.part");
        assert!(fixture.root.join("new.part").exists());
    }

    assert!(!fixture.root.join("new.part").exists());
}

// oracle: mutant forgetting a failed cleanup and allowing a reconnect before it succeeds.
#[test]
fn download_sink_retries_failed_cleanup_before_reconnect() {
    let fixture = Fixture::new();
    let factory = WindowsDownloadSinkFactory::new(&fixture.root).expect("pin download directory");
    let mut backend = factory.build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");
    let path = fixture.root.join("new.part");
    let mut permissions = std::fs::metadata(&path).expect("file metadata").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions.clone()).expect("make cleanup fail");

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .expect("complete close");
    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    assert!(path.exists());
    let mut reconnect = factory.build();
    assert!(reconnect.restore_drive(1).is_err());
    let mut create = create_request(r"\other.part", CreateDisposition::FILE_CREATE);
    create.desired_access = DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE;
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
        .expect("complete create while cleanup is blocked");
    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    assert!(!fixture.root.join("other.part").exists());

    permissions.set_readonly(false);
    std::fs::set_permissions(&path, permissions).expect("allow cleanup");
    reconnect.restore_drive(1).expect("retry cleanup on reconnect");
    assert!(!path.exists());
}

// oracle: mutant retaining an unfinished file or its ID after device removal.
#[test]
fn download_sink_drive_removal_cleans_open_files() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");

    backend.remove_drive(1).expect("remove download device");
    assert!(!fixture.root.join("new.part").exists());
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"stale".to_vec(),
        }))
        .expect("complete stale write");
    assert_eq!(response_status(&response), NtStatus::INVALID_HANDLE);
    backend.add_drive(1).expect("reactivate download device");
}

// oracle: mutant writing to or publishing a transfer after another hard link is added.
#[test]
fn download_sink_refuses_hard_linked_transfer_files() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");
    let path = fixture.root.join("new.part");
    let alias = fixture.sandbox.join("linked.part");
    std::fs::hard_link(&path, &alias).expect("link the open download");

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"not allowed".to_vec(),
        }))
        .expect("complete linked-file write");
    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::Rename(FileRenameInformation {
                    replace_if_exists: Boolean::False,
                    file_name: r"\finished.txt".to_owned(),
                }),
            },
        ))
        .expect("complete linked-file publication");
    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .expect("close linked transfer");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(!path.exists());
    assert_eq!(std::fs::read(alias).expect("read hard link"), b"");
}

// oracle: mutant following a Windows junction when pinning the approved directory.
#[test]
fn download_sink_rejects_junction_as_approved_directory() {
    let fixture = Fixture::new();
    let junction = fixture.sandbox.join("junction");
    let result = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&fixture.root)
        .output()
        .expect("run junction setup");
    assert!(result.status.success(), "junction setup failed: {result:?}");

    assert!(WindowsDownloadSinkFactory::new(&junction).is_err());
    assert!(WindowsDownloadSinkFactory::new(&junction.join("root")).is_err());
    std::fs::remove_dir(&junction).expect("remove test junction");
}

// oracle: mutant applying the open-file budget per connection instead of across the factory.
#[test]
fn download_sink_shares_open_file_budget_across_connections() {
    let fixture = Fixture::new();
    let factory = WindowsDownloadSinkFactory::new(&fixture.root).expect("pin download directory");
    let mut first = factory.build();
    let mut second = factory.build();
    first.restore_drive(1).expect("activate first device");
    second.restore_drive(1).expect("activate second device");
    let mut first_file = None;
    for index in 0..512 {
        let id = create_download(&mut first, &format!(r"\first-{index}.part"));
        first_file.get_or_insert(id);
        create_download(&mut second, &format!(r"\second-{index}.part"));
    }

    let mut request = create_request(r"\overflow.part", CreateDisposition::FILE_CREATE);
    request.desired_access = DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE;
    let response = second
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request.clone()))
        .expect("complete over-budget create");
    assert_eq!(response_status(&response), NtStatus::INSUFFICIENT_RESOURCES);
    assert!(!fixture.root.join("overflow.part").exists());
    let close = first
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(first_file.expect("first file ID"), MajorFunction::Close),
        }))
        .expect("close one download");
    assert_eq!(response_status(&close), NtStatus::SUCCESS);
    let response = second
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
        .expect("complete create after close");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
}

// oracle: mutant permitting writes, deletions, or additional renames after publication.
#[test]
fn download_sink_keeps_published_files_immutable() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let file_id = create_download(&mut backend, r"\new.part");
    let write = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"original".to_vec(),
        }))
        .expect("complete write");
    assert_eq!(response_status(&write), NtStatus::SUCCESS);
    let rename = |name: &str| {
        ServerDriveIoRequest::ServerDriveSetInformationRequest(ServerDriveSetInformationRequest {
            device_io_request: request_header(file_id, MajorFunction::SetInformation),
            set_buffer: FileInformationClass::Rename(FileRenameInformation {
                replace_if_exists: Boolean::False,
                file_name: name.to_owned(),
            }),
        })
    };
    let publication = backend
        .handle_drive_io_request(rename(r"\finished.txt"))
        .expect("complete publication");
    assert_eq!(response_status(&publication), NtStatus::SUCCESS);

    let write = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"modified".to_vec(),
        }))
        .expect("complete post-publication write");
    assert_eq!(response_status(&write), NtStatus::ACCESS_DENIED);
    let deletion = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request_header(file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 }),
            },
        ))
        .expect("complete post-publication deletion");
    assert_eq!(response_status(&deletion), NtStatus::ACCESS_DENIED);
    let rename = backend
        .handle_drive_io_request(rename(r"\other.txt"))
        .expect("complete post-publication rename");
    assert_eq!(response_status(&rename), NtStatus::ACCESS_DENIED);
    let close = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .expect("complete close");
    assert_eq!(response_status(&close), NtStatus::SUCCESS);
    assert_eq!(
        std::fs::read(fixture.root.join("finished.txt")).expect("read published file"),
        b"original"
    );
    assert!(!fixture.root.join("other.txt").exists());
}

// oracle: mutant creating a file after the client revokes directory creation access.
#[test]
fn download_sink_observes_directory_acl_changes() {
    let fixture = Fixture::new();
    let mut backend = WindowsDownloadSinkFactory::new(&fixture.root)
        .expect("pin download directory")
        .build();
    backend.restore_drive(1).expect("activate download device");
    let identity = std::process::Command::new("whoami.exe")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .expect("query Windows user");
    assert!(identity.status.success());
    let identity = String::from_utf8_lossy(&identity.stdout);
    let identity = identity
        .split('"')
        .find(|field| field.starts_with("S-1-"))
        .expect("Windows user SID");
    let identity = format!("*{identity}");
    let denied = std::process::Command::new("icacls.exe")
        .arg(&fixture.root)
        .arg("/deny")
        .arg(format!("{identity}:(WD)"))
        .output()
        .expect("deny directory file creation");
    assert!(denied.status.success(), "icacls deny failed: {denied:?}");

    let mut request = create_request(r"\new.part", CreateDisposition::FILE_CREATE);
    request.desired_access = DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE;
    let response = backend.handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request));
    let restored = std::process::Command::new("icacls.exe")
        .arg(&fixture.root)
        .arg("/remove:d")
        .arg(&identity)
        .output()
        .expect("restore directory ACL");
    assert!(restored.status.success(), "icacls restore failed: {restored:?}");
    let response = response.expect("complete create");
    assert_eq!(response_status(&response), NtStatus::ACCESS_DENIED);
}

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
    assert!(
        scheduled.is_empty(),
        "directory notification was completed immediately with {:?}",
        response_status(&scheduled),
    );

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
fn issue_2233_colliding_fixture_timestamps_do_not_share_cleanup() {
    // oracle: regression #2233
    let timestamp = SystemTime::now();
    let first = Fixture::with_timestamp(timestamp);
    let second = Fixture::with_timestamp(timestamp);

    assert_ne!(first.sandbox, second.sandbox, "fixtures must own distinct sandboxes");
    drop(first);
    assert!(
        second.root.is_dir(),
        "another fixture's cleanup must preserve this root"
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

fn create_download(backend: &mut impl RdpdrBackend, path: &str) -> u32 {
    let mut request = create_request(path, CreateDisposition::FILE_CREATE);
    request.desired_access =
        DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE | DesiredAccess::DELETE | DesiredAccess::SYNCHRONIZE;
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
        .expect("complete download create");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    response_file_id(&response)
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
        Self::with_timestamp(SystemTime::now())
    }

    fn with_timestamp(timestamp: SystemTime) -> Self {
        let workspace = std::env::current_dir().expect("current workspace directory");
        let volume_root = workspace
            .ancestors()
            .last()
            .expect("workspace has a volume root")
            .to_owned();
        let parent = workspace.join("target");
        std::fs::create_dir_all(&parent).expect("create fixture parent");
        let prefix = format!(
            "rdpdr-native-{}-{}",
            std::process::id(),
            timestamp
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after Unix epoch")
                .as_nanos()
        );
        let mut suffix = 0u64;
        let sandbox = loop {
            let sandbox = parent.join(format!("{prefix}-{suffix}"));
            match std::fs::create_dir(&sandbox) {
                Ok(()) => break sandbox,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    suffix = suffix.checked_add(1).expect("fixture directory suffix exhausted");
                }
                Err(error) => panic!("create fixture sandbox: {error}"),
            }
        };
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
