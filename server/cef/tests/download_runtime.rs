#![cfg(feature = "runtime")]

use std::error::Error as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ironrdp_cef_server::download::{Limits, runtime::Downloads};
use ironrdp_core::{DecodeError, DecodeErrorKind, ReadCursor, WriteCursor, encode_vec};
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_rdpdr::pdu::efs::*;
use ironrdp_rdpdr::server::{NoopRdpdrServerBackend, RdpdrServer};
use ironrdp_server::{RdpdrServerBackend, RdpdrServerMessage, ServerEvent};
use ironrdp_svc::StaticVirtualChannel;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

struct Fixture {
    downloads: Downloads,
    worker: Option<JoinHandle<()>>,
    root: PathBuf,
    backend: Box<dyn RdpdrServerBackend>,
    events: UnboundedReceiver<ServerEvent>,
}

impl Fixture {
    fn new(limits: Limits) -> Self {
        Self::with_enabled(limits, true)
    }

    fn with_enabled(limits: Limits, enabled: bool) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = PathBuf::from("target").join(format!(
            "download-check-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let (downloads, worker) = Downloads::new(&root, enabled, limits).unwrap();
        let mut factory = downloads.factory();
        let (sender, events) = unbounded_channel();
        factory.set_sender(sender);
        let backend = factory.build_backend();
        Self {
            downloads,
            worker: Some(worker),
            root,
            backend,
            events,
        }
    }

    fn announce(&mut self) {
        let mut devices = Devices::new();
        devices.add_drive(42, "DL".to_owned());
        assert_eq!(self.backend.on_device_announce(&devices.clone_inner()), [(42, true)]);
    }

    fn stage(&self, id: u32, payload: &[u8]) -> PathBuf {
        let path = self.downloads.begin(id, "fixture.bin").unwrap();
        std::fs::write(&path, payload).unwrap();
        self.downloads.progress(id, payload.len() as u64, true, false);
        path
    }

    fn event(&mut self) -> ServerEvent {
        self.event_with_timeout(Duration::from_secs(3))
    }

    fn event_with_timeout(&mut self, timeout: Duration) -> ServerEvent {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(event) = self.events.try_recv() {
                return event;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for a server event: {}",
                self.downloads.status()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn request(&mut self) -> RdpdrServerMessage {
        match self.event() {
            ServerEvent::Rdpdr(request) => request,
            other => panic!("unexpected event: {other:?}"),
        }
    }

    fn created(&mut self) {
        let RdpdrServerMessage::Create {
            device_id,
            path,
            create_disposition,
            create_options,
            desired_access,
        } = self.request()
        else {
            panic!("expected exclusive create");
        };
        assert_eq!(device_id, 42);
        assert!(path.starts_with(".ircef-") && path.ends_with(".part"));
        assert!(!path.contains(['/', '\\']));
        assert_eq!(create_disposition, CreateDisposition::FILE_CREATE);
        assert_eq!(create_options, CreateOptions::FILE_NON_DIRECTORY_FILE);
        assert!(!desired_access.intersects(DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY));
        self.backend
            .on_create_complete(&DeviceCreateResponse {
                device_io_reply: header(NtStatus::SUCCESS),
                file_id: 7,
                information: Information::empty(),
            })
            .unwrap();
    }

    fn flushed_empty(&mut self) {
        assert!(matches!(
            self.request(),
            RdpdrServerMessage::FlushBuffers {
                device_id: 42,
                file_id: 7
            }
        ));
        self.backend
            .on_flush_buffers_complete(&DeviceFlushBuffersResponse {
                device_io_response: header(NtStatus::SUCCESS),
            })
            .unwrap();
    }

    fn renamed(&mut self, status: NtStatus) {
        let RdpdrServerMessage::SetInformation {
            device_id,
            file_id,
            set_buffer,
        } = self.request()
        else {
            panic!("expected rename");
        };
        assert_eq!((device_id, file_id), (42, 7));
        let length = set_buffer.size() as u32;
        let FileInformationClass::Rename(rename) = set_buffer else {
            panic!("expected rename information");
        };
        assert_eq!(rename.file_name, "fixture.bin");
        assert_eq!(rename.replace_if_exists, Boolean::False);
        self.backend
            .on_set_information_complete(
                &ClientDriveSetInformationResponse::decode(header(status), &mut ReadCursor::new(&length.to_le_bytes()))
                    .unwrap(),
            )
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.downloads.stop();
        self.worker.take().unwrap().join().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn header(status: NtStatus) -> DeviceIoResponse {
    DeviceIoResponse {
        device_id: 42,
        completion_id: 0,
        io_status: status,
    }
}

fn wait_until(check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(Instant::now() < deadline, "download state did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

// oracle: mutant retaining an oversized client message before the RDPDR server parses it.
#[test]
fn server_rejects_oversized_fragment_before_reassembly() {
    for enabled in [false, true] {
        let fixture = Fixture::with_enabled(Limits::default(), enabled);
        let backend = fixture.downloads.factory().build_backend();
        let mut channel = StaticVirtualChannel::new(RdpdrServer::new(backend));
        let mut fragment = encode_vec(&ChannelPduHeader {
            length: 1_048_577,
            flags: ChannelControlFlags::FLAG_FIRST,
        })
        .unwrap();
        fragment.push(0);

        let error = channel.process(&fragment).expect_err("oversized message");
        let source = error
            .source()
            .and_then(|source| source.downcast_ref::<DecodeError>())
            .expect("static-channel decode error");
        assert!(
            matches!(
                source.kind(),
                DecodeErrorKind::InvalidField {
                    field: "length",
                    reason: "static channel message exceeds configured maximum size",
                    ..
                }
            ),
            "downloads enabled: {enabled}"
        );
    }
}

// oracle: mutant applying the CEF-specific inbound limit to other RDPDR server backends.
#[test]
fn other_server_backends_keep_their_unlimited_reassembly_default() {
    let mut channel = StaticVirtualChannel::new(RdpdrServer::new(Box::new(NoopRdpdrServerBackend)));
    let mut fragment = encode_vec(&ChannelPduHeader {
        length: 1_048_577,
        flags: ChannelControlFlags::FLAG_FIRST,
    })
    .unwrap();
    fragment.push(0);

    assert!(channel.process(&fragment).unwrap().is_empty());
}

// oracle: mutant rejecting the configured maximum RDPDR message size.
#[test]
fn server_accepts_fragment_at_reassembly_limit() {
    let fixture = Fixture::new(Limits::default());
    let backend = fixture.downloads.factory().build_backend();
    let mut channel = StaticVirtualChannel::new(RdpdrServer::new(backend));
    let mut fragment = encode_vec(&ChannelPduHeader {
        length: 1_048_576,
        flags: ChannelControlFlags::FLAG_FIRST,
    })
    .unwrap();
    fragment.push(0);

    assert!(channel.process(&fragment).unwrap().is_empty());
}

// oracle: mutant completing delivery before acknowledged writes, flush, rename, and close.
#[test]
fn multi_chunk_delivery_retries_collision_and_completes_only_after_close() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let payload: Vec<u8> = (0..150_000).map(|offset| (offset % 251) as u8).collect();
    let stage = fixture.stage(1, &payload);
    fixture.created();
    let mut received = Vec::new();
    loop {
        match fixture.request() {
            RdpdrServerMessage::Write {
                device_id,
                file_id,
                data,
                offset,
            } => {
                assert_eq!((device_id, file_id, offset), (42, 7, received.len() as u64));
                assert!(data.len() <= 65536);
                received.extend_from_slice(&data);
                fixture
                    .backend
                    .on_write_complete(&DeviceWriteResponse {
                        device_io_reply: header(NtStatus::SUCCESS),
                        length: data.len() as u32,
                    })
                    .unwrap();
            }
            RdpdrServerMessage::FlushBuffers { device_id, file_id } => {
                assert_eq!((device_id, file_id), (42, 7));
                fixture
                    .backend
                    .on_flush_buffers_complete(&DeviceFlushBuffersResponse {
                        device_io_response: header(NtStatus::SUCCESS),
                    })
                    .unwrap();
                break;
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }
    assert_eq!(received, payload);
    for (name, status) in [
        ("fixture.bin", NtStatus::OBJECT_NAME_COLLISION),
        ("fixture (1).bin", NtStatus::SUCCESS),
    ] {
        let RdpdrServerMessage::SetInformation {
            device_id,
            file_id,
            set_buffer,
        } = fixture.request()
        else {
            panic!("expected non-replacing rename");
        };
        assert_eq!((device_id, file_id), (42, 7));
        let length = set_buffer.size() as u32;
        let FileInformationClass::Rename(rename) = set_buffer else {
            panic!("expected rename information");
        };
        assert_eq!(rename.file_name, name);
        assert_eq!(rename.replace_if_exists, Boolean::False);
        fixture
            .backend
            .on_set_information_complete(
                &ClientDriveSetInformationResponse::decode(header(status), &mut ReadCursor::new(&length.to_le_bytes()))
                    .unwrap(),
            )
            .unwrap();
    }
    assert!(matches!(
        fixture.request(),
        RdpdrServerMessage::Close {
            device_id: 42,
            file_id: 7
        }
    ));
    assert!(fixture.downloads.status().contains("Published:"));
    assert!(!fixture.downloads.status().contains("Completed:"));
    fixture
        .backend
        .on_close_complete(&DeviceCloseResponse {
            device_io_response: header(NtStatus::SUCCESS),
        })
        .unwrap();
    wait_until(|| fixture.downloads.status().contains("Completed: fixture (1).bin"));
    wait_until(|| !stage.exists());
}

// oracle: mutant publishing a partial file after a short write.
#[test]
fn short_write_fails_and_fences_the_connection() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let stage = fixture.stage(1, b"binary\0payload");
    fixture.created();
    assert!(matches!(fixture.request(), RdpdrServerMessage::Write { .. }));
    fixture
        .backend
        .on_write_complete(&DeviceWriteResponse {
            device_io_reply: header(NtStatus::SUCCESS),
            length: 1,
        })
        .unwrap();
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
    wait_until(|| fixture.downloads.status().contains("short download write"));
    assert!(fixture.downloads.available().is_err());
    wait_until(|| !stage.exists());
}

// oracle: mutant reusing an unacknowledged request slot after cancellation or disconnect.
#[test]
fn pending_cancel_requires_connection_fencing_and_cleans_staging() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let stage = fixture.stage(1, b"payload");
    assert!(matches!(fixture.request(), RdpdrServerMessage::Create { .. }));
    fixture.downloads.cancel_all();
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
    assert!(fixture.downloads.disconnected());
    fixture
        .backend
        .on_create_complete(&DeviceCreateResponse {
            device_io_reply: header(NtStatus::SUCCESS),
            file_id: 7,
            information: Information::FILE_CREATED,
        })
        .unwrap();
    assert!(fixture.downloads.available().is_err());
    wait_until(|| !stage.exists());
    assert!(!fixture.downloads.status().contains("Completed:"));
}

// oracle: mutant selecting a device with the wrong name or type.
#[test]
fn rejects_missing_wrong_name_wrong_type_and_duplicate_drives() {
    let mut fixture = Fixture::new(Limits::default());
    assert!(fixture.downloads.begin(1, "fixture.bin").is_err());
    let mut devices = Devices::new();
    devices.add_drive(1, "OTHER".to_owned());
    let mut announcement = ClientDeviceListAnnounce {
        device_list: devices.clone_inner(),
    };
    let mut bytes = vec![0; announcement.size()];
    announcement.encode(&mut WriteCursor::new(&mut bytes)).unwrap();
    bytes[4..8].copy_from_slice(&0x20u32.to_le_bytes());
    bytes[12..20].copy_from_slice(b"DL\0\0\0\0\0\0");
    announcement = ClientDeviceListAnnounce::decode(&mut ReadCursor::new(&bytes)).unwrap();
    assert_eq!(
        fixture.backend.on_device_announce(&announcement.device_list),
        [(1, false)]
    );
    assert_eq!(fixture.backend.on_device_announce(&devices.clone_inner()), [(1, false)]);
    fixture.announce();
    devices.add_drive(2, "DL".to_owned());
    assert_eq!(
        fixture.backend.on_device_announce(&devices.clone_inner()),
        [(1, false), (2, false)]
    );
}

// oracle: mutant retaining a previous device or browser download across reinitialization.
#[test]
fn reinitialization_accepts_the_drive_again_and_cancels_old_staging() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let old = fixture.downloads.begin(1, "old.bin").unwrap();
    std::fs::write(&old, b"old").unwrap();
    fixture.backend.on_reinitialize(&[42]).unwrap();
    assert!(fixture.downloads.should_cancel(1));
    fixture.downloads.progress(1, 3, false, true);
    fixture.announce();

    let next = fixture.stage(2, b"");
    fixture.created();
    fixture.flushed_empty();
    fixture.renamed(NtStatus::SUCCESS);
    assert!(matches!(fixture.request(), RdpdrServerMessage::Close { .. }));
    fixture
        .backend
        .on_close_complete(&DeviceCloseResponse {
            device_io_response: header(NtStatus::SUCCESS),
        })
        .unwrap();
    wait_until(|| fixture.downloads.status().contains("#2 Completed: fixture.bin"));
    wait_until(|| !old.exists() && !next.exists());
}

// oracle: mutant reinitializing a channel while a client request is unacknowledged.
#[test]
fn reinitialization_with_pending_irp_fails_and_fences_the_connection() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    fixture.stage(1, b"");
    assert!(matches!(fixture.request(), RdpdrServerMessage::Create { .. }));
    assert!(fixture.backend.on_reinitialize(&[42]).is_err());
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
    assert!(fixture.downloads.disconnected());
    assert!(fixture.downloads.available().is_err());
}

// oracle: mutant sending a queued request after a removed drive is reannounced.
#[test]
fn removing_a_drive_with_a_queued_request_fails_the_channel() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    fixture.stage(1, b"");
    wait_until(|| !fixture.events.is_empty());

    assert!(fixture.backend.try_on_device_remove(&[42]).is_err());
    assert!(matches!(
        fixture.event(),
        ServerEvent::Rdpdr(RdpdrServerMessage::Create { .. })
    ));
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
    assert!(fixture.downloads.disconnected());
}

// oracle: mutant enforcing staging limits only from CEF progress callbacks.
#[test]
fn staging_monitor_accounts_actual_bytes_and_cancels_overshoot() {
    let mut fixture = Fixture::new(Limits {
        file_bytes: 4,
        session_bytes: 6,
        concurrent: 2,
    });
    fixture.announce();
    let stage = fixture.downloads.begin(1, "fixture.bin").unwrap();
    std::fs::write(&stage, b"12345").unwrap();
    wait_until(|| fixture.downloads.should_cancel(1));
    assert!(fixture.downloads.status().contains("observed 5 B"));
    fixture.downloads.progress(1, 0, false, true);
    wait_until(|| !stage.exists());
    assert!(fixture.events.try_recv().is_err());
    assert!(!fixture.downloads.status().contains("Completed:"));
}

// oracle: mutant releasing a concurrency slot before staging cleanup succeeds.
#[test]
fn failed_staging_cleanup_keeps_the_download_slot_until_removed() {
    let mut fixture = Fixture::new(Limits {
        concurrent: 1,
        ..Limits::default()
    });
    fixture.announce();
    let stage = fixture.downloads.begin(1, "fixture.bin").unwrap();
    std::fs::create_dir(&stage).unwrap();
    fixture.downloads.progress(1, 0, false, true);
    wait_until(|| fixture.downloads.status().contains("failed to remove staged download"));
    assert!(fixture.downloads.begin(2, "next.bin").is_err());
    std::fs::remove_dir(&stage).unwrap();
    wait_until(|| fixture.downloads.begin(2, "next.bin").is_ok());
}

// oracle: mutant hiding a failure when a concurrent download completes.
#[test]
fn concurrent_failure_remains_visible_after_another_download_succeeds() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let failed = fixture.downloads.begin(1, "failed.bin").unwrap();
    std::fs::write(&failed, []).unwrap();
    let successful = fixture.downloads.begin(2, "fixture.bin").unwrap();
    std::fs::write(&successful, []).unwrap();
    fixture.downloads.progress(1, 0, false, true);
    wait_until(|| !failed.exists());
    wait_until(|| fixture.downloads.status().contains("#1 Failed:"));
    let status = fixture.downloads.status();
    assert!(status.contains("#1 Failed:"), "{status}");
    assert!(status.contains("#2 Browser download"), "{status}");

    fixture.downloads.progress(2, 0, true, false);
    fixture.created();
    fixture.flushed_empty();
    fixture.renamed(NtStatus::SUCCESS);
    assert!(matches!(fixture.request(), RdpdrServerMessage::Close { .. }));
    fixture
        .backend
        .on_close_complete(&DeviceCloseResponse {
            device_io_response: header(NtStatus::SUCCESS),
        })
        .unwrap();
    wait_until(|| fixture.downloads.status().contains("#2 Completed: fixture.bin"));
    let status = fixture.downloads.status();
    assert!(status.contains("#1 Failed:"), "{status}");
    assert!(status.contains("#2 Completed: fixture.bin"), "{status}");
}

// oracle: mutant completing an empty download without flush, publication, and close.
#[test]
fn empty_download_requires_all_commit_acknowledgments() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let stage = fixture.stage(1, b"");
    fixture.created();
    fixture.flushed_empty();
    fixture.renamed(NtStatus::SUCCESS);
    assert!(matches!(fixture.request(), RdpdrServerMessage::Close { .. }));
    fixture
        .backend
        .on_close_complete(&DeviceCloseResponse {
            device_io_response: header(NtStatus::SUCCESS),
        })
        .unwrap();
    wait_until(|| fixture.downloads.status().contains("Completed: fixture.bin"));
    wait_until(|| !stage.exists());
}

// oracle: mutant reporting completion after a failed rename.
#[test]
fn refused_rename_fails_without_sending_close_as_success() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let stage = fixture.stage(1, b"");
    fixture.created();
    fixture.flushed_empty();
    fixture.renamed(NtStatus::ACCESS_DENIED);
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
    wait_until(|| fixture.downloads.status().contains("Failed:"));
    assert!(!fixture.downloads.status().contains("Completed:"));
    wait_until(|| !stage.exists());
}

// oracle: spec MS-RDPEFS §2.2.3.4.9 requires the response Length to match the request.
#[test]
fn mismatched_rename_acknowledgment_fails_delivery() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let stage = fixture.stage(1, b"");
    fixture.created();
    fixture.flushed_empty();
    let RdpdrServerMessage::SetInformation { set_buffer, .. } = fixture.request() else {
        panic!("expected rename");
    };
    let wrong_length = u32::try_from(set_buffer.size() + 1).unwrap();
    fixture
        .backend
        .on_set_information_complete(
            &ClientDriveSetInformationResponse::decode(
                header(NtStatus::SUCCESS),
                &mut ReadCursor::new(&wrong_length.to_le_bytes()),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
    wait_until(|| {
        fixture
            .downloads
            .status()
            .contains("invalid rename acknowledgment length")
    });
    assert!(!fixture.downloads.status().contains("Completed:"));
    wait_until(|| !stage.exists());
}

// oracle: mutant reporting completion before a successful close acknowledgment.
#[test]
fn refused_close_reports_that_the_file_was_already_published() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let stage = fixture.stage(1, b"");
    fixture.created();
    fixture.flushed_empty();
    fixture.renamed(NtStatus::SUCCESS);
    assert!(matches!(fixture.request(), RdpdrServerMessage::Close { .. }));
    fixture
        .backend
        .on_close_complete(&DeviceCloseResponse {
            device_io_response: header(NtStatus::ACCESS_DENIED),
        })
        .unwrap();
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
    wait_until(|| fixture.downloads.status().contains("client file was published"));
    assert!(!fixture.downloads.status().contains("Completed:"));
    wait_until(|| !stage.exists());
}

// oracle: mutant treating an acknowledged close as unconfirmed at disconnect.
#[test]
fn disconnect_after_close_acknowledgment_does_not_require_fencing() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let stage = fixture.stage(1, b"");
    fixture.created();
    fixture.flushed_empty();
    fixture.renamed(NtStatus::SUCCESS);
    assert!(matches!(fixture.request(), RdpdrServerMessage::Close { .. }));
    fixture
        .backend
        .on_close_complete(&DeviceCloseResponse {
            device_io_response: header(NtStatus::SUCCESS),
        })
        .unwrap();
    assert!(!fixture.downloads.disconnected());
    wait_until(|| fixture.downloads.status().contains("Completed: fixture.bin"));
    wait_until(|| !stage.exists());
}

// oracle: mutant accepting an acknowledgment from the wrong device.
#[test]
fn acknowledgment_for_another_device_keeps_the_request_fenced() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    fixture.stage(1, b"");
    assert!(matches!(fixture.request(), RdpdrServerMessage::Create { .. }));
    let mut response = header(NtStatus::SUCCESS);
    response.device_id = 43;
    fixture
        .backend
        .on_create_complete(&DeviceCreateResponse {
            device_io_reply: response,
            file_id: 7,
            information: Information::FILE_CREATED,
        })
        .unwrap();
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
    assert!(fixture.downloads.disconnected());
}

// oracle: mutant accepting an acknowledgment for the wrong operation.
#[test]
fn acknowledgment_for_another_operation_keeps_the_request_fenced() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    fixture.stage(1, b"");
    assert!(matches!(fixture.request(), RdpdrServerMessage::Create { .. }));
    fixture
        .backend
        .on_write_complete(&DeviceWriteResponse {
            device_io_reply: header(NtStatus::SUCCESS),
            length: 0,
        })
        .unwrap();
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
    assert!(fixture.downloads.disconnected());
}

// oracle: mutant accepting an acknowledgment from a previous connection.
#[test]
fn stale_acknowledgment_does_not_complete_a_reconnected_request() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let first = fixture.stage(1, b"");
    fixture.created();
    fixture.flushed_empty();
    fixture.renamed(NtStatus::SUCCESS);
    assert!(matches!(fixture.request(), RdpdrServerMessage::Close { .. }));
    fixture
        .backend
        .on_close_complete(&DeviceCloseResponse {
            device_io_response: header(NtStatus::SUCCESS),
        })
        .unwrap();
    assert!(!fixture.downloads.disconnected());

    let factory = fixture.downloads.factory();
    let mut old_backend = std::mem::replace(&mut fixture.backend, factory.build_backend());
    fixture.announce();
    let second = fixture.stage(2, b"");
    assert!(matches!(fixture.request(), RdpdrServerMessage::Create { .. }));
    old_backend
        .on_create_complete(&DeviceCreateResponse {
            device_io_reply: header(NtStatus::SUCCESS),
            file_id: 99,
            information: Information::FILE_CREATED,
        })
        .unwrap();
    fixture
        .backend
        .on_create_complete(&DeviceCreateResponse {
            device_io_reply: header(NtStatus::SUCCESS),
            file_id: 7,
            information: Information::FILE_CREATED,
        })
        .unwrap();
    fixture.flushed_empty();
    fixture.renamed(NtStatus::SUCCESS);
    assert!(matches!(fixture.request(), RdpdrServerMessage::Close { .. }));
    fixture
        .backend
        .on_close_complete(&DeviceCloseResponse {
            device_io_response: header(NtStatus::SUCCESS),
        })
        .unwrap();
    wait_until(|| fixture.downloads.status().contains("#2 Completed: fixture.bin"));
    wait_until(|| !first.exists() && !second.exists());
}

// oracle: mutant ignoring staging limits while another transfer waits for the client.
#[test]
fn staging_monitor_keeps_running_while_delivery_is_waiting_for_an_ack() {
    let mut fixture = Fixture::new(Limits {
        file_bytes: 4,
        session_bytes: 6,
        concurrent: 2,
    });
    fixture.announce();
    fixture.stage(1, b"1234");
    assert!(matches!(fixture.request(), RdpdrServerMessage::Create { .. }));
    let second = fixture.downloads.begin(2, "second.bin").unwrap();
    std::fs::write(&second, b"12345").unwrap();
    wait_until(|| fixture.downloads.should_cancel(2));
    fixture.downloads.cancel_all();
    fixture.downloads.progress(2, 5, false, true);
    assert!(matches!(fixture.event(), ServerEvent::Disconnect(_)));
}

// oracle: mutant reusing a request slot after an acknowledgment timeout.
#[test]
fn missing_acknowledgment_times_out_without_reusing_the_slot() {
    let mut fixture = Fixture::new(Limits::default());
    fixture.announce();
    let stage = fixture.stage(1, b"payload");
    assert!(matches!(fixture.request(), RdpdrServerMessage::Create { .. }));
    assert!(matches!(
        fixture.event_with_timeout(Duration::from_secs(33)),
        ServerEvent::Disconnect(_)
    ));
    wait_until(|| fixture.downloads.status().contains("acknowledgment timed out"));
    assert!(fixture.downloads.disconnected());
    assert!(fixture.downloads.begin(2, "other.bin").is_err());
    wait_until(|| !stage.exists());
}
