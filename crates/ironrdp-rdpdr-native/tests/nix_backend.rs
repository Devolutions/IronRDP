#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use ironrdp_core::{DecodeError, DecodeErrorKind, ReadCursor, encode_vec};
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCreateRequest,
    DeviceFlushBuffersRequest, DeviceIoRequest, DeviceReadRequest, DeviceWriteRequest, FileAttributes,
    FileDispositionInformation, FileInformationClass, FileInformationClassLevel, FileRenameInformation,
    FileStandardInformation, LockOperation, MajorFunction, MinorFunction, NtStatus, RdpLockInfo,
    ServerDeviceAnnounceResponse, ServerDriveIoRequest, ServerDriveLockControlRequest,
    ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest, ServerDriveSetInformationRequest,
    SharedAccess, VersionAndIdPdu, VersionAndIdPduKind,
};
use ironrdp_rdpdr::{Rdpdr, RdpdrBackend, RdpdrBackendFactory};
use ironrdp_rdpdr_native::backend::{NixRdpdrBackend, NixRdpdrBackendFactory};
use ironrdp_rdpdr_native::printer::PrintTarget;
use ironrdp_svc::{StaticVirtualChannel, SvcMessage, SvcProcessor as _};
use rstest::rstest;
use tracing as _;

// The read limit of the Windows drive backend.
const MAX_READ_LENGTH: u32 = 1_024 * 1_024;

fn download_backend(fixture: &Fixture) -> NixRdpdrBackend {
    NixRdpdrBackend::new_download_sink(fixture.share.to_str().expect("UTF-8 directory").to_owned())
        .expect("open download sink")
}

fn download_backend_with_two_devices(fixture: &Fixture) -> Box<dyn RdpdrBackend> {
    NixRdpdrBackendFactory::new(fixture.share.to_str().unwrap().to_owned())
        .with_download_sink()
        .unwrap()
        .with_drive(1, "first".to_owned())
        .with_drive(2, "second".to_owned())
        .build_rdpdr_backend()
        .unwrap()
        .into_parts()
        .0
}

fn download_create(backend: &mut dyn RdpdrBackend, path: &str) -> Vec<SvcMessage> {
    backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(DeviceCreateRequest {
            desired_access: DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE
                | DesiredAccess::DELETE
                | DesiredAccess::SYNCHRONIZE,
            create_options: CreateOptions::FILE_NON_DIRECTORY_FILE,
            ..create_request(path, CreateDisposition::FILE_CREATE)
        }))
        .expect("complete download create")
}

// oracle: spec MS-RDPEFS §2.2.1.4.4 (wire layout); mutant retaining a fragment over the download limit.
#[test]
fn download_sink_bounds_inbound_static_channel_reassembly() {
    use std::error::Error as _;

    const LIMIT: u32 = 1_048_576 + 4 /* SharedHeader */ + 20 /* DeviceIoRequest */ + 32 /* Write header */;
    let fixture = Fixture::new();
    let channel = |backend| {
        StaticVirtualChannel::new(
            Rdpdr::new(Box::new(backend), "test".to_owned()).with_drives(Some(vec![(1, "DL".to_owned())])),
        )
    };
    let fragment = |length| {
        let mut bytes = encode_vec(&ChannelPduHeader {
            length,
            flags: ChannelControlFlags::FLAG_FIRST,
        })
        .unwrap();
        bytes.push(0);
        bytes
    };

    let mut at_limit = channel(download_backend(&fixture));
    assert!(at_limit.process(&fragment(LIMIT)).unwrap().is_empty());
    let mut oversized = channel(download_backend(&fixture));
    let error = oversized.process(&fragment(LIMIT + 1)).expect_err("oversized message");
    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<DecodeError>())
        .expect("static-channel decode error");
    assert!(matches!(
        source.kind(),
        DecodeErrorKind::InvalidField {
            field: "length",
            reason: "static channel message exceeds configured maximum size",
            ..
        }
    ));

    let mut unrestricted = channel(NixRdpdrBackend::new(fixture.share.to_str().unwrap().to_owned()));
    assert!(unrestricted.process(&fragment(LIMIT + 1)).unwrap().is_empty());
}

// oracle: spec MS-RDPEFS §2.2.1.1 and §2.2.1.4.4 (wire layout); mutant rejecting an exact-limit write.
#[test]
fn download_sink_accepts_an_exact_limit_fragmented_write() {
    let fixture = Fixture::new();
    let mut rdpdr = Rdpdr::new(Box::new(download_backend(&fixture)), "test".to_owned())
        .with_drives(Some(vec![(1, "DL".to_owned())]));
    let announce = VersionAndIdPdu {
        version_major: 1,
        version_minor: 12,
        client_id: 0x1234,
        kind: VersionAndIdPduKind::ServerAnnounceRequest,
    };
    let responses = rdpdr
        .process(&encode_vec(&RdpdrPdu::VersionAndIdPdu(announce.clone())).unwrap())
        .unwrap();
    let reply = responses[0].encode_unframed_pdu().unwrap();
    let client_id = u32::from_le_bytes(reply[8..12].try_into().unwrap());
    rdpdr
        .process(
            &encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
                client_id,
                kind: VersionAndIdPduKind::ServerClientIdConfirm,
                ..announce
            }))
            .unwrap(),
        )
        .unwrap();
    rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    rdpdr
        .process(
            &encode_vec(&RdpdrPdu::ServerDeviceAnnounceResponse(ServerDeviceAnnounceResponse {
                device_id: 1,
                result_code: NtStatus::SUCCESS,
            }))
            .unwrap(),
        )
        .unwrap();

    let backend = rdpdr.downcast_backend_mut::<NixRdpdrBackend>().unwrap();
    let created = download_create(backend, ".exact.part");
    let file_id = response_file_id(&created);
    let data = vec![0xa5; 1_048_576];
    let mut request = encode_vec(&RdpdrPdu::DeviceIoRequest(request_header(
        file_id,
        MajorFunction::Write,
    )))
    .unwrap();
    request.extend_from_slice(&1_048_576u32.to_le_bytes()); // Length
    request.extend_from_slice(&0u64.to_le_bytes()); // Offset
    request.extend_from_slice(&[0; 20]); // Padding
    request.extend_from_slice(&data);
    assert_eq!(request.len(), 1_048_632);

    let mut channel = StaticVirtualChannel::new(rdpdr);
    let chunks = request.chunks(1600);
    let count = chunks.len();
    for (index, chunk) in chunks.enumerate() {
        let mut flags = ChannelControlFlags::empty();
        if index == 0 {
            flags |= ChannelControlFlags::FLAG_FIRST;
        }
        if index + 1 == count {
            flags |= ChannelControlFlags::FLAG_LAST;
        }
        let mut frame = encode_vec(&ChannelPduHeader {
            length: 1_048_632,
            flags,
        })
        .unwrap();
        frame.extend_from_slice(chunk);
        let responses = channel.process(&frame).expect("bounded fragment");
        if index + 1 == count {
            assert_eq!(response_status(&responses), NtStatus::SUCCESS);
        } else {
            assert!(responses.is_empty());
        }
    }
    assert_eq!(std::fs::read(fixture.share.join(".exact.part")).unwrap(), data);
}

// oracle: mutant applying the download-only message cap to other configured device types.
#[test]
fn mixed_device_channels_keep_their_existing_reassembly_behavior() {
    let fixture = Fixture::new();
    let fragment = {
        let mut bytes = encode_vec(&ChannelPduHeader {
            length: 16_777_272,
            flags: ChannelControlFlags::FLAG_FIRST,
        })
        .unwrap();
        bytes.push(0);
        bytes
    };
    let printer_backend = download_backend(&fixture).with_printer(2, PrintTarget::DefaultPrinter);
    let mut printer = StaticVirtualChannel::new(
        Rdpdr::new(Box::new(printer_backend), "test".to_owned())
            .with_drives(Some(vec![(1, "DL".to_owned())]))
            .with_printer(2, "printer".to_owned()),
    );
    assert!(printer.process(&fragment).unwrap().is_empty());

    let mut smartcard = StaticVirtualChannel::new(
        Rdpdr::new(Box::new(download_backend(&fixture)), "test".to_owned())
            .with_drives(Some(vec![(1, "DL".to_owned())]))
            .with_smartcard(0),
    );
    assert!(smartcard.process(&fragment).unwrap().is_empty());
}

// oracle: mutant removing other devices' handles or committed files with a rejected device.
// Published downloads survive; incomplete files on that device do not.
#[test]
fn download_sink_removes_only_the_rejected_devices_incomplete_files() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let partial = download_create(&mut backend, ".first.part");
    assert_eq!(response_status(&partial), NtStatus::SUCCESS);
    let published = download_create(&mut backend, ".published.part");
    assert_eq!(response_status(&published), NtStatus::SUCCESS);
    let published_id = response_file_id(&published);
    assert_eq!(
        response_status(&write(&mut backend, published_id, 0, b"saved")),
        NtStatus::SUCCESS
    );
    assert_eq!(
        response_status(&set_information(
            &mut backend,
            published_id,
            rename_to("published.txt", false),
        )),
        NtStatus::SUCCESS
    );

    let mut request = create_request(".second.part", CreateDisposition::FILE_CREATE);
    request.device_io_request.device_id = 2;
    request.desired_access = DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE | DesiredAccess::DELETE;
    request.create_options = CreateOptions::FILE_NON_DIRECTORY_FILE;
    let second = create_with(&mut backend, request);
    assert_eq!(response_status(&second), NtStatus::SUCCESS);
    let second_id = response_file_id(&second);

    assert!(backend.remove_drive(1).expect("remove drive").is_empty());
    assert!(!fixture.share.join(".first.part").exists());
    assert_eq!(std::fs::read(fixture.share.join("published.txt")).unwrap(), b"saved");
    let mut write_request = request_header(second_id, MajorFunction::Write);
    write_request.device_id = 2;
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: write_request,
            offset: 0,
            write_data: b"other drive".to_vec(),
        }))
        .expect("write remaining drive");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(
        std::fs::read(fixture.share.join(".second.part")).unwrap(),
        b"other drive"
    );
}

// oracle: spec MS-RDPEFS §2.2.1.4 (each request targets a specific device).
#[test]
fn download_sink_rejects_writes_with_another_devices_file_id() {
    let fixture = Fixture::new();
    let mut backend = download_backend_with_two_devices(&fixture);
    let created = download_create(backend.as_mut(), ".first.part");
    let file_id = response_file_id(&created);
    let mut request = request_header(file_id, MajorFunction::Write);
    request.device_id = 2;

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request,
            offset: 0,
            write_data: b"wrong device".to_vec(),
        }))
        .unwrap();
    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(std::fs::read(fixture.share.join(".first.part")).unwrap(), b"");
}

// oracle: spec MS-RDPEFS §2.2.1.4 (each request targets a specific device).
#[test]
fn download_sink_rejects_closes_with_another_devices_file_id() {
    let fixture = Fixture::new();
    let mut backend = download_backend_with_two_devices(&fixture);
    let created = download_create(backend.as_mut(), ".first.part");
    let file_id = response_file_id(&created);
    let mut request = request_header(file_id, MajorFunction::Close);
    request.device_id = 2;

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request,
        }))
        .unwrap();
    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"first device".to_vec(),
        }))
        .unwrap();
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert_eq!(
        std::fs::read(fixture.share.join(".first.part")).unwrap(),
        b"first device"
    );
}

// oracle: spec MS-RDPEFS §2.2.1.4 (each request targets a specific device).
#[test]
fn download_sink_rejects_queries_with_another_devices_file_id() {
    let fixture = Fixture::new();
    let mut backend = download_backend_with_two_devices(&fixture);
    let created = download_create(backend.as_mut(), ".first.part");
    let file_id = response_file_id(&created);
    let mut request = request_header(file_id, MajorFunction::QueryInformation);
    request.device_id = 2;

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryInformationRequest(
            ServerDriveQueryInformationRequest {
                device_io_request: request,
                file_info_class_lvl: FileInformationClassLevel::FILE_STANDARD_INFORMATION,
            },
        ))
        .unwrap();
    assert_ne!(response_status(&response), NtStatus::SUCCESS);
}

// oracle: spec MS-RDPEFS §2.2.1.4 (each request targets a specific device).
#[test]
fn download_sink_rejects_flushes_with_another_devices_file_id() {
    let fixture = Fixture::new();
    let mut backend = download_backend_with_two_devices(&fixture);
    let created = download_create(backend.as_mut(), ".first.part");
    let file_id = response_file_id(&created);
    let mut request = request_header(file_id, MajorFunction::FlushBuffers);
    request.device_id = 2;

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceFlushBuffersRequest(
            DeviceFlushBuffersRequest {
                device_io_request: request,
            },
        ))
        .unwrap();
    assert_ne!(response_status(&response), NtStatus::SUCCESS);
}

// oracle: spec MS-RDPEFS §2.2.1.4 (each request targets a specific device).
#[test]
fn download_sink_rejects_renames_with_another_devices_file_id() {
    let fixture = Fixture::new();
    let mut backend = download_backend_with_two_devices(&fixture);
    let created = download_create(backend.as_mut(), ".first.part");
    let file_id = response_file_id(&created);
    let mut request = request_header(file_id, MajorFunction::SetInformation);
    request.device_id = 2;

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: request,
                set_buffer: rename_to("finished.txt", false),
            },
        ))
        .unwrap();
    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    assert!(fixture.share.join(".first.part").exists());
    assert!(!fixture.share.join("finished.txt").exists());
}

// oracle: spec MS-RDPEFS §3.1.5.2 (an unknown FileId cannot identify an open device).
#[test]
fn download_sink_rejects_closes_for_unowned_file_ids() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);

    let response = close(&mut backend, u32::MAX);

    assert_ne!(response_status(&response), NtStatus::SUCCESS);
}

// oracle: spec MS-RDPEFS §2.2.1.5.1 and §2.2.3.4.1
#[test]
fn download_sink_reports_exclusive_create_information() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let response = download_create(&mut backend, ".created.part");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let bytes = response[0].encode_unframed_pdu().unwrap();
    assert!((20..=21).contains(&bytes.len()));
    assert_eq!(bytes.get(20).copied().unwrap_or(0), 0);
}

// oracle: mutant removing a published file or retaining unpublished files after teardown.
#[test]
fn download_sink_publishes_exact_bytes_and_preserves_committed_files() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let created = download_create(&mut backend, ".ircef-one.part");
    assert_eq!(response_status(&created), NtStatus::SUCCESS);
    let file_id = response_file_id(&created);
    assert_eq!(
        response_status(&write(&mut backend, file_id, 0, b"\0download\xff")),
        NtStatus::SUCCESS
    );
    assert_eq!(query_standard_information(&mut backend, file_id).end_of_file, 10);
    let flushed = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceFlushBuffersRequest(
            ironrdp_rdpdr::pdu::efs::DeviceFlushBuffersRequest {
                device_io_request: request_header(file_id, MajorFunction::FlushBuffers),
            },
        ))
        .expect("flush");
    assert_eq!(response_status(&flushed), NtStatus::SUCCESS);
    assert_eq!(
        response_status(&set_information(&mut backend, file_id, rename_to("résumé.bin", false))),
        NtStatus::SUCCESS
    );
    assert_eq!(
        response_status(&write(&mut backend, file_id, 0, b"overwrite")),
        NtStatus::ACCESS_DENIED
    );
    assert_eq!(
        response_status(&set_information(&mut backend, file_id, delete_pending(true))),
        NtStatus::ACCESS_DENIED
    );
    assert_eq!(response_status(&close(&mut backend, file_id)), NtStatus::SUCCESS);
    backend.reset().expect("reset");
    drop(backend);
    assert_eq!(
        std::fs::read(fixture.share.join("résumé.bin")).expect("download"),
        b"\0download\xff"
    );
    assert!(!fixture.share.join(".ircef-one.part").exists());
}

// oracle: mutant replacing an existing file during publication.
#[test]
fn download_sink_collision_and_replace_requests_preserve_existing_files() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let created = download_create(&mut backend, ".ircef-one.part");
    let file_id = response_file_id(&created);
    std::fs::write(fixture.share.join("existing"), b"keep").expect("existing destination");
    assert_eq!(
        response_status(&set_information(&mut backend, file_id, rename_to("existing", false))),
        NtStatus::OBJECT_NAME_COLLISION
    );
    assert_eq!(
        response_status(&set_information(&mut backend, file_id, rename_to("existing", true))),
        NtStatus::ACCESS_DENIED
    );
    assert_eq!(
        response_status(&download_create(&mut backend, "existing")),
        NtStatus::OBJECT_NAME_COLLISION
    );
    assert_eq!(
        std::fs::read(fixture.share.join("existing")).expect("existing"),
        b"keep"
    );
    drop(backend);
    assert!(!fixture.share.join(".ircef-one.part").exists());
}

// oracle: mutant allowing uploads, enumeration, pre-existing mutation, traversal, or links.
#[test]
fn download_sink_denies_reads_listings_and_nonexclusive_opens() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let created = download_create(&mut backend, ".ircef-one.part");
    let file_id = response_file_id(&created);
    assert_eq!(
        response_status(&read(&mut backend, file_id, 0, 10)),
        NtStatus::ACCESS_DENIED
    );
    assert_eq!(
        response_status(&query_directory(&mut backend, file_id, "*", true)),
        NtStatus::ACCESS_DENIED
    );
    for disposition in [
        CreateDisposition::FILE_OPEN,
        CreateDisposition::FILE_OPEN_IF,
        CreateDisposition::FILE_OVERWRITE,
        CreateDisposition::FILE_OVERWRITE_IF,
        CreateDisposition::FILE_SUPERSEDE,
    ] {
        assert_eq!(
            response_status(&create_with(
                &mut backend,
                DeviceCreateRequest {
                    desired_access: DesiredAccess::GENERIC_WRITE | DesiredAccess::DELETE,
                    ..create_request(".ircef-one.part", disposition)
                }
            )),
            NtStatus::ACCESS_DENIED
        );
    }
    for path in ["../escape", r"..\escape", "/absolute", r"sub\file", ""] {
        assert_ne!(
            response_status(&download_create(&mut backend, path)),
            NtStatus::SUCCESS,
            "{path}"
        );
    }
    std::fs::write(fixture.sandbox.join("sentinel"), b"keep").expect("sentinel");
    std::os::unix::fs::symlink(fixture.sandbox.join("sentinel"), fixture.share.join("link")).expect("link");
    assert_ne!(
        response_status(&download_create(&mut backend, "link")),
        NtStatus::SUCCESS
    );
    assert_eq!(
        response_status(&set_information(&mut backend, file_id, rename_to("link", false))),
        NtStatus::OBJECT_NAME_COLLISION
    );
    assert_eq!(
        std::fs::read(fixture.sandbox.join("sentinel")).expect("sentinel"),
        b"keep"
    );
}

// oracle: mutant cleaning another channel's files or following a replaced root path.
#[test]
fn download_sink_cleanup_stays_in_the_pinned_directory() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    assert_eq!(
        response_status(&download_create(&mut backend, ".ircef-reset.part")),
        NtStatus::SUCCESS
    );
    backend.reset().expect("reset");
    assert!(!fixture.share.join(".ircef-reset.part").exists());
    assert_eq!(
        response_status(&download_create(&mut backend, ".ircef-drop.part")),
        NtStatus::SUCCESS
    );
    let pinned = fixture.sandbox.join("moved");
    std::fs::rename(&fixture.share, &pinned).expect("move root");
    std::fs::create_dir(&fixture.share).expect("replacement root");
    std::fs::write(fixture.share.join(".ircef-drop.part"), b"unrelated").expect("unrelated");
    let created = download_create(&mut backend, ".ircef-pinned.part");
    assert_eq!(response_status(&created), NtStatus::SUCCESS);
    let file_id = response_file_id(&created);
    assert_eq!(
        response_status(&set_information(&mut backend, file_id, rename_to("pinned", false))),
        NtStatus::SUCCESS
    );
    assert!(pinned.join("pinned").is_file());
    assert!(!fixture.share.join("pinned").exists());
    drop(backend);
    assert!(!pinned.join(".ircef-drop.part").exists());
    assert_eq!(
        std::fs::read(fixture.share.join(".ircef-drop.part")).expect("unrelated"),
        b"unrelated"
    );
}

// oracle: mutant reopening a substituted download root when a connection starts.
#[test]
fn download_factory_pins_approved_directory_before_connecting() {
    let fixture = Fixture::new();
    let factory = NixRdpdrBackendFactory::new(fixture.share.to_str().unwrap().to_owned())
        .with_download_sink()
        .unwrap()
        .with_drive(1, "DL".to_owned());
    let approved = fixture.sandbox.join("approved");
    let outside = fixture.sandbox.join("outside");
    std::fs::rename(&fixture.share, &approved).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, &fixture.share).unwrap();

    let (mut backend, _) = factory.build_rdpdr_backend().unwrap().into_parts();
    let response = download_create(backend.as_mut(), ".ircef-pinned.part");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    assert!(approved.join(".ircef-pinned.part").exists());
    assert!(!outside.join(".ircef-pinned.part").exists());
}

// oracle: mutant applying the open-file limit separately to channels from the same factory.
#[test]
fn download_factory_shares_the_file_limit_across_channels() {
    use nix::sys::resource::{Resource, getrlimit};

    let fixture = Fixture::new();
    let factory = NixRdpdrBackendFactory::new(fixture.share.to_str().unwrap().to_owned())
        .with_download_sink()
        .unwrap()
        .with_drive(1, "DL".to_owned());
    let (mut first, _) = factory.build_rdpdr_backend().unwrap().into_parts();
    let (mut second, _) = factory.build_rdpdr_backend().unwrap().into_parts();
    let (soft_limit, _) = getrlimit(Resource::RLIMIT_NOFILE).unwrap();
    let limit = usize::try_from(soft_limit / 4).unwrap_or(usize::MAX).min(1_024);
    assert!(2 <= limit, "the test needs at least two download handles");
    let start = std::sync::Barrier::new(2);

    let (first_results, second_results) = std::thread::scope(|scope| {
        let open = |backend: &mut dyn RdpdrBackend, prefix: &str| {
            start.wait();
            (0..=limit)
                .map(|index| {
                    let response = download_create(backend, &format!(".{prefix}-{index}.part"));
                    let status = response_status(&response);
                    (
                        status,
                        (status == NtStatus::SUCCESS).then(|| response_file_id(&response)),
                    )
                })
                .collect::<Vec<_>>()
        };
        let first_backend = first.as_mut();
        let second_backend = second.as_mut();
        let first = scope.spawn(move || open(first_backend, "first"));
        let second = scope.spawn(move || open(second_backend, "second"));
        (first.join().unwrap(), second.join().unwrap())
    });

    let results = first_results.iter().chain(&second_results);
    let accepted = results
        .clone()
        .filter(|(status, _)| *status == NtStatus::SUCCESS)
        .count();
    assert!(0 < accepted, "no download was accepted");
    assert!(
        accepted <= limit,
        "{accepted} downloads exceeded the shared limit {limit}"
    );
    assert!(
        results
            .clone()
            .all(|(status, _)| *status == NtStatus::SUCCESS || *status == NtStatus::INSUFFICIENT_RESOURCES)
    );

    let (owner, other, file_id) = if let Some(file_id) = first_results.iter().find_map(|(_, id)| *id) {
        (first.as_mut(), second.as_mut(), file_id)
    } else {
        let file_id = second_results.iter().find_map(|(_, id)| *id).unwrap();
        (second.as_mut(), first.as_mut(), file_id)
    };
    owner
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .unwrap();
    assert_eq!(
        response_status(&download_create(other, ".after-release.part")),
        NtStatus::SUCCESS
    );
}

// oracle: mutant retaining a shared handle reservation after an exclusive create fails.
#[test]
fn download_factory_releases_failed_create_reservations() {
    let fixture = Fixture::new();
    let factory = NixRdpdrBackendFactory::new(fixture.share.to_str().unwrap().to_owned())
        .with_download_sink()
        .unwrap()
        .with_drive(1, "DL".to_owned());
    let (mut backend, _) = factory.build_rdpdr_backend().unwrap().into_parts();
    assert_eq!(
        response_status(&download_create(backend.as_mut(), ".occupied.part")),
        NtStatus::SUCCESS
    );
    for _ in 0..1_025 {
        assert_eq!(
            response_status(&download_create(backend.as_mut(), ".occupied.part")),
            NtStatus::OBJECT_NAME_COLLISION
        );
    }
    assert_eq!(
        response_status(&download_create(backend.as_mut(), ".available.part")),
        NtStatus::SUCCESS
    );
}

#[cfg(target_os = "macos")]
fn with_fstat_fault(test_name: &str, test: impl FnOnce(&std::path::Path, &std::path::Path)) {
    const CHILD: &str = "IRONRDP_TEST_FSTAT_CHILD";

    if let Some(share) = std::env::var_os(CHILD) {
        let share = PathBuf::from(share);
        let marker = PathBuf::from(std::env::var_os("IRONRDP_TEST_FSTAT_MARKER").unwrap());
        test(&share, &marker);
        return;
    }

    let fixture = Fixture::new();
    let share = fixture.share.canonicalize().unwrap();
    let marker = fixture.sandbox.join("fstat-fault");
    let library = fixture.sandbox.join("fstat-fault.dylib");
    std::fs::write(&marker, []).unwrap();
    let compiler = std::process::Command::new("cc")
        .args(["-std=c11", "-dynamiclib", "-O2"])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fstat_fault.c"))
        .arg("-o")
        .arg(&library)
        .output()
        .expect("compile fstat fault library");
    assert!(
        compiler.status.success(),
        "could not compile fstat fault library: {}",
        String::from_utf8_lossy(&compiler.stderr)
    );

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env("DYLD_INSERT_LIBRARIES", &library)
        .env(CHILD, &share)
        .env("IRONRDP_TEST_FSTAT_PATH", share.join(".metadata-fault.part"))
        .env("IRONRDP_TEST_FSTAT_MARKER", marker)
        .output()
        .expect("run fstat fault test");
    assert!(
        output.status.success(),
        "fstat fault test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// oracle: mutant forgetting a file when metadata fails after exclusive creation.
#[cfg(target_os = "macos")]
#[test]
fn download_sink_cleans_up_after_post_create_metadata_failure() {
    with_fstat_fault(
        "download_sink_cleans_up_after_post_create_metadata_failure",
        |share, marker| {
            let mut backend = NixRdpdrBackend::new_download_sink(share.to_str().unwrap().to_owned()).unwrap();

            assert_ne!(
                response_status(&download_create(&mut backend, ".metadata-fault.part")),
                NtStatus::SUCCESS
            );
            assert!(
                !std::fs::read(marker).unwrap().is_empty(),
                "fstat fault was not reached"
            );
            std::fs::remove_file(marker).unwrap();
            backend.reset().unwrap();
            assert!(
                !share.join(".metadata-fault.part").exists(),
                "failed create left a file"
            );
        },
    );
}

// oracle: mutant forgetting an unidentifiable file or deleting a replacement after a failed create.
#[cfg(target_os = "macos")]
#[test]
fn download_factory_retains_post_create_cleanup_across_connections() {
    with_fstat_fault(
        "download_factory_retains_post_create_cleanup_across_connections",
        |share, marker| {
            let factory = NixRdpdrBackendFactory::new(share.to_str().unwrap().to_owned())
                .with_download_sink()
                .unwrap()
                .with_drive(1, "DL".to_owned());
            let (mut backend, _) = factory.build_rdpdr_backend().unwrap().into_parts();
            let original = share.join(".metadata-fault.part");
            let saved = share.join(".metadata-fault.saved");

            assert_ne!(
                response_status(&download_create(backend.as_mut(), ".metadata-fault.part")),
                NtStatus::SUCCESS
            );
            assert!(
                !std::fs::read(marker).unwrap().is_empty(),
                "fstat fault was not reached"
            );
            drop(backend);
            assert!(factory.build_rdpdr_backend().is_err());

            std::fs::rename(&original, &saved).unwrap();
            std::fs::write(&original, b"unrelated").unwrap();
            std::fs::remove_file(marker).unwrap();
            assert!(factory.build_rdpdr_backend().is_err());
            assert_eq!(std::fs::read(&original).unwrap(), b"unrelated");

            std::fs::remove_file(&original).unwrap();
            std::fs::rename(&saved, &original).unwrap();
            let (mut next, _) = factory.build_rdpdr_backend().unwrap().into_parts();
            assert!(!original.exists());
            assert_eq!(
                response_status(&download_create(next.as_mut(), ".next.part")),
                NtStatus::SUCCESS
            );
        },
    );
}

// oracle: mutant retaining the reservation after a failed create is cleaned up.
#[cfg(target_os = "macos")]
#[test]
fn download_sink_releases_post_create_failure_reservations() {
    with_fstat_fault(
        "download_sink_releases_post_create_failure_reservations",
        |share, marker| {
            let mut backend = NixRdpdrBackend::new_download_sink(share.to_str().unwrap().to_owned()).unwrap();

            for _ in 0..1_025 {
                std::fs::write(marker, []).unwrap();
                assert_ne!(
                    response_status(&download_create(&mut backend, ".metadata-fault.part")),
                    NtStatus::SUCCESS
                );
                assert!(
                    !std::fs::read(marker).unwrap().is_empty(),
                    "fstat fault was not reached"
                );
                std::fs::remove_file(marker).unwrap();
                backend.reset().unwrap();
            }
            assert_eq!(
                response_status(&download_create(&mut backend, ".next.part")),
                NtStatus::SUCCESS
            );
        },
    );
}

// oracle: mutant forgetting an unpublished file or reporting successful cleanup after failure.
#[test]
fn download_factory_retries_failed_cleanup_after_reconnect() {
    let fixture = Fixture::new();
    let factory = NixRdpdrBackendFactory::new(fixture.share.to_str().unwrap().to_owned())
        .with_download_sink()
        .unwrap()
        .with_drive(1, "DL".to_owned());
    let (mut backend, _) = factory.build_rdpdr_backend().unwrap().into_parts();
    let original = fixture.share.join(".ircef-partial.part");
    let saved = fixture.share.join(".ircef-saved.part");
    let response = download_create(backend.as_mut(), ".ircef-partial.part");
    assert_eq!(response_status(&response), NtStatus::SUCCESS);
    let file_id = response_file_id(&response);
    std::fs::rename(&original, &saved).unwrap();
    std::fs::write(&original, b"unrelated").unwrap();

    let response = backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .unwrap();
    assert_ne!(response_status(&response), NtStatus::SUCCESS);
    drop(backend);
    assert_eq!(std::fs::read(&original).unwrap(), b"unrelated");
    assert!(factory.build_rdpdr_backend().is_err());
    std::fs::remove_file(&original).unwrap();
    std::fs::rename(&saved, &original).unwrap();

    let _next = factory.build_rdpdr_backend().unwrap();
    assert!(!original.exists());
}

// oracle: mutant discarding other pending cleanups when one channel's cleanup succeeds.
#[test]
fn download_factory_retains_each_failed_cleanup_across_channels() {
    let fixture = Fixture::new();
    let factory = NixRdpdrBackendFactory::new(fixture.share.to_str().unwrap().to_owned())
        .with_download_sink()
        .unwrap()
        .with_drive(1, "DL".to_owned());
    let (mut first, _) = factory.build_rdpdr_backend().unwrap().into_parts();
    let (mut second, _) = factory.build_rdpdr_backend().unwrap().into_parts();
    let first_response = download_create(first.as_mut(), ".first.part");
    let second_response = download_create(second.as_mut(), ".second.part");
    assert_eq!(response_status(&first_response), NtStatus::SUCCESS);
    assert_eq!(response_status(&second_response), NtStatus::SUCCESS);
    let first_id = response_file_id(&first_response);
    let second_id = response_file_id(&second_response);
    for name in ["first", "second"] {
        std::fs::rename(
            fixture.share.join(format!(".{name}.part")),
            fixture.share.join(format!(".{name}.saved")),
        )
        .unwrap();
        std::fs::write(fixture.share.join(format!(".{name}.part")), b"unrelated").unwrap();
    }
    for (backend, file_id) in [(first.as_mut(), first_id), (second.as_mut(), second_id)] {
        backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
                device_io_request: request_header(file_id, MajorFunction::Close),
            }))
            .unwrap();
    }
    std::fs::remove_file(fixture.share.join(".first.part")).unwrap();
    std::fs::rename(fixture.share.join(".first.saved"), fixture.share.join(".first.part")).unwrap();

    assert!(factory.build_rdpdr_backend().is_err());
    assert!(!fixture.share.join(".first.part").exists());
    assert_eq!(std::fs::read(fixture.share.join(".second.part")).unwrap(), b"unrelated");
    std::fs::remove_file(fixture.share.join(".second.part")).unwrap();
    std::fs::rename(fixture.share.join(".second.saved"), fixture.share.join(".second.part")).unwrap();

    let _next = factory.build_rdpdr_backend().unwrap();
    assert!(!fixture.share.join(".second.part").exists());
}

// oracle: mutant allowing more incomplete files while an earlier cleanup cannot succeed.
#[test]
fn download_sink_retries_cleanup_before_creating_another_file() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let original = fixture.share.join(".ircef-partial.part");
    let saved = fixture.share.join(".ircef-saved.part");
    let created = download_create(&mut backend, ".ircef-partial.part");
    assert_eq!(response_status(&created), NtStatus::SUCCESS);
    let file_id = response_file_id(&created);
    std::fs::rename(&original, &saved).unwrap();
    std::fs::write(&original, b"unrelated").unwrap();
    close(&mut backend, file_id);

    let rejected = download_create(&mut backend, ".ircef-next.part");
    assert_ne!(response_status(&rejected), NtStatus::SUCCESS);
    assert!(!fixture.share.join(".ircef-next.part").exists());
    assert_eq!(std::fs::read(&original).unwrap(), b"unrelated");

    std::fs::remove_file(&original).unwrap();
    std::fs::rename(&saved, &original).unwrap();
    let accepted = download_create(&mut backend, ".ircef-next.part");
    assert_eq!(response_status(&accepted), NtStatus::SUCCESS);
    assert!(!original.exists());
}

// oracle: mutant reporting a successful reset while a failed download cleanup remains unresolved.
#[test]
fn download_sink_reset_reports_and_retries_pending_cleanup() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let original = fixture.share.join(".ircef-partial.part");
    let saved = fixture.share.join(".ircef-saved.part");
    let created = download_create(&mut backend, ".ircef-partial.part");
    let file_id = response_file_id(&created);
    std::fs::rename(&original, &saved).unwrap();
    std::fs::write(&original, b"unrelated").unwrap();
    close(&mut backend, file_id);

    assert!(backend.reset().is_err());
    assert_eq!(std::fs::read(&original).unwrap(), b"unrelated");
    std::fs::remove_file(&original).unwrap();
    std::fs::rename(&saved, &original).unwrap();
    backend.reset().unwrap();
    assert!(!original.exists());
}

// oracle: mutant reporting successful device removal while an incomplete download cannot be cleaned up.
#[test]
fn download_sink_device_removal_reports_and_retries_failed_cleanup() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let original = fixture.share.join(".ircef-partial.part");
    let saved = fixture.share.join(".ircef-saved.part");
    let created = download_create(&mut backend, ".ircef-partial.part");
    assert_eq!(response_status(&created), NtStatus::SUCCESS);
    std::fs::rename(&original, &saved).unwrap();
    std::fs::write(&original, b"unrelated").unwrap();

    assert!(backend.remove_drive(1).is_err());
    assert_eq!(std::fs::read(&original).unwrap(), b"unrelated");
    std::fs::remove_file(&original).unwrap();
    std::fs::rename(&saved, &original).unwrap();
    backend.remove_drive(1).unwrap();
    assert!(!original.exists());
}

// oracle: mutant retaining a cleanup block after the owned file was removed by the local user.
#[test]
fn download_sink_unblocks_after_owned_file_is_removed() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let original = fixture.share.join(".ircef-partial.part");
    let saved = fixture.share.join(".ircef-saved.part");
    let created = download_create(&mut backend, ".ircef-partial.part");
    let file_id = response_file_id(&created);
    std::fs::rename(&original, &saved).unwrap();
    std::fs::write(&original, b"unrelated").unwrap();
    close(&mut backend, file_id);
    std::fs::remove_file(&saved).unwrap();

    let accepted = download_create(&mut backend, ".ircef-next.part");
    assert_eq!(response_status(&accepted), NtStatus::SUCCESS);
    assert_eq!(std::fs::read(&original).unwrap(), b"unrelated");
}

// oracle: mutant accepting a write over 1 MiB or a file offset beyond 1 GiB.
#[test]
fn download_sink_bounds_write_lengths_and_offsets() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    let created = download_create(&mut backend, ".ircef-one.part");
    let file_id = response_file_id(&created);
    assert_eq!(
        response_status(&write(&mut backend, file_id, 0, &vec![0; 1_024 * 1_024 + 1])),
        NtStatus::INVALID_PARAMETER
    );
    assert_eq!(
        response_status(&write(&mut backend, file_id, u64::MAX, b"x")),
        NtStatus::INVALID_PARAMETER
    );
    assert_eq!(
        response_status(&write(&mut backend, file_id, 1_024 * 1_024 * 1_024, b"x")),
        NtStatus::INVALID_PARAMETER
    );
    assert_eq!(
        response_status(&write(&mut backend, file_id, 0, &vec![0; 1_024 * 1_024])),
        NtStatus::SUCCESS
    );
    assert_eq!(
        std::fs::metadata(fixture.share.join(".ircef-one.part"))
            .expect("metadata")
            .len(),
        1_024 * 1_024
    );
}

// oracle: mutant opening an existing special file or link as a writable transfer.
#[test]
fn download_sink_refuses_existing_links_directories_and_special_files() {
    let fixture = Fixture::new();
    std::fs::write(fixture.share.join("existing"), b"keep").expect("existing");
    std::fs::hard_link(fixture.share.join("existing"), fixture.share.join("hardlink")).expect("hard link");
    std::fs::create_dir(fixture.share.join("directory")).expect("directory");
    nix::unistd::mkfifo(&fixture.share.join("fifo"), nix::sys::stat::Mode::S_IRWXU).expect("FIFO");
    let mut backend = download_backend(&fixture);
    for name in ["existing", "hardlink", "directory", "fifo"] {
        assert_ne!(
            response_status(&download_create(&mut backend, name)),
            NtStatus::SUCCESS,
            "{name}"
        );
    }
    assert_eq!(
        std::fs::read(fixture.share.join("existing")).expect("existing"),
        b"keep"
    );
    assert_eq!(
        response_status(&set_information(&mut backend, 999, delete_pending(true))),
        NtStatus::NO_SUCH_FILE
    );
}

// oracle: mutant deleting a replacement that the channel did not create.
#[test]
fn download_sink_cleanup_does_not_remove_a_replacement_file() {
    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    assert_eq!(
        response_status(&download_create(&mut backend, ".ircef-one.part")),
        NtStatus::SUCCESS
    );
    std::fs::rename(fixture.share.join(".ircef-one.part"), fixture.sandbox.join("moved")).expect("move transfer");
    std::fs::write(fixture.share.join(".ircef-one.part"), b"unrelated").expect("replacement");
    drop(backend);
    assert_eq!(
        std::fs::read(fixture.share.join(".ircef-one.part")).expect("replacement"),
        b"unrelated"
    );
}

// oracle: mutant allowing both racing publishers to replace the same destination.
#[test]
fn download_sink_publication_never_replaces_a_racing_destination() {
    use std::io::Write as _;
    use std::sync::{Arc, Barrier};

    let fixture = Fixture::new();
    let mut backend = download_backend(&fixture);
    for index in 0..32 {
        let temporary = format!(".ircef-{index}.part");
        let destination = format!("download-{index}");
        let created = download_create(&mut backend, &temporary);
        assert_eq!(response_status(&created), NtStatus::SUCCESS);
        let file_id = response_file_id(&created);
        assert_eq!(
            response_status(&write(&mut backend, file_id, 0, b"remote")),
            NtStatus::SUCCESS
        );
        let barrier = Arc::new(Barrier::new(2));
        let writer = std::thread::spawn({
            let barrier = Arc::clone(&barrier);
            let path = fixture.share.join(&destination);
            move || {
                barrier.wait();
                match std::fs::OpenOptions::new().create_new(true).write(true).open(path) {
                    Ok(mut file) => {
                        file.write_all(b"local").expect("write local winner");
                        true
                    }
                    Err(error) => {
                        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
                        false
                    }
                }
            }
        });
        barrier.wait();
        let status = response_status(&set_information(&mut backend, file_id, rename_to(&destination, false)));
        let local_won = writer.join().expect("local writer");
        assert_eq!(
            status,
            if local_won {
                NtStatus::OBJECT_NAME_COLLISION
            } else {
                NtStatus::SUCCESS
            }
        );
        close(&mut backend, file_id);
        assert_eq!(
            std::fs::read(fixture.share.join(destination)).expect("winner"),
            if local_won {
                b"local".as_slice()
            } else {
                b"remote".as_slice()
            }
        );
        assert!(!fixture.share.join(temporary).exists());
    }
}

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
    backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveLockControlRequest(
            ServerDriveLockControlRequest {
                device_io_request: DeviceIoRequest {
                    completion_id,
                    ..request_header(file_id, MajorFunction::LockControl)
                },
                operation,
                wait,
                locks: vec![RdpLockInfo { length, offset }],
            },
        ))
        .expect("complete lock control")
}

fn read(backend: &mut NixRdpdrBackend, file_id: u32, offset: u64, length: u32) -> Vec<SvcMessage> {
    backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
            device_io_request: request_header(file_id, MajorFunction::Read),
            length,
            offset,
        }))
        .expect("complete read")
}

fn write(backend: &mut NixRdpdrBackend, file_id: u32, offset: u64, data: &[u8]) -> Vec<SvcMessage> {
    backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset,
            write_data: data.to_vec(),
        }))
        .expect("complete write")
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

fn close(backend: &mut NixRdpdrBackend, file_id: u32) -> Vec<SvcMessage> {
    backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: request_header(file_id, MajorFunction::Close),
        }))
        .expect("complete close")
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
const COMPLETION_ID: Range<usize> = 8..12;
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
