// FIXME: tests in this module can probably be rewritten to be much shorter using the ironrdp-client crate.

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use anyhow::Result;
use ironrdp::connector;
use ironrdp::core::{Encode as _, ReadCursor, encode_vec};
use ironrdp::dvc::DrdynvcClient;
use ironrdp::echo::client::EchoClient;
use ironrdp::pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp::pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::CompressionType as PduCompressionType;
use ironrdp::pdu::rdp::headers::CompressionFlags;
use ironrdp::pdu::{self, gcc};
#[cfg(windows)]
use ironrdp::rdpdr::backend::RdpdrBackendFactory as _;
use ironrdp::rdpdr::pdu::efs::{
    Capabilities, CoreCapability, CoreCapabilityKind, DeviceIoRequest, DeviceIoResponse, MajorFunction, MinorFunction,
    NtStatus, ServerDeviceAnnounceResponse, VERSION_MAJOR, VERSION_MINOR_13, VersionAndIdPdu, VersionAndIdPduKind,
};
use ironrdp::rdpdr::pdu::{PacketId, RdpdrPdu, SharedHeader};
use ironrdp::rdpdr::{NoopRdpdrBackend, Rdpdr};
use ironrdp::rdpsnd::client::{NoopRdpsndBackend, Rdpsnd};
use ironrdp::server::{
    self, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, RdpServerInputHandler, ServerEvent, TlsIdentityCtx,
};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{self, ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp::svc::{StaticChannelSet, SvcMessage, SvcProcessor, SvcServerProcessor};
use ironrdp_async::{Framed, FramedWrite as _};
use ironrdp_bulk::{BulkCompressor, CompressionType as BulkCompressionType, flags as bulk_flags};
use ironrdp_testsuite_extra as _;
use ironrdp_tls::TlsStream;
use ironrdp_tokio::TokioStream;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, oneshot};
use tracing::debug;
#[cfg(windows)]
use uuid::Uuid;

const DESKTOP_WIDTH: u16 = 1024;
const DESKTOP_HEIGHT: u16 = 768;
const USERNAME: &str = "";
const PASSWORD: &str = "";

#[tokio::test]
async fn test_client_server() {
    client_server(
        default_client_config(),
        |stage, _activation_factory, framed, _display_tx| async { (stage, framed) },
    )
    .await
}

#[tokio::test]
async fn test_deactivation_reactivation() {
    let client_config = default_client_config();
    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        client_config.desktop_size.width,
        client_config.desktop_size.height,
    );
    client_server(
        client_config,
        |mut stage, activation_factory, mut framed, display_tx| async move {
            display_tx
                .send(DisplayUpdate::Resize(DesktopSize {
                    width: 2048,
                    height: 2048,
                }))
                .unwrap();
            {
                let (action, payload) = framed.read_pdu().await.expect("valid PDU");
                let outputs = stage.process(&mut image, action, &payload).expect("stage process");
                let out = outputs.into_iter().next().unwrap();
                match out {
                    ActiveStageOutput::DeactivateAll => {
                        // TODO: factor this out in common client code
                        // Execute the Deactivation-Reactivation Sequence:
                        // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
                        debug!("Received Server Deactivate All PDU, executing Deactivation-Reactivation Sequence");
                        let mut connection_activation = activation_factory.create();
                        let mut buf = pdu::WriteBuf::new();
                        'activation_seq: loop {
                            let written = ironrdp_async::single_sequence_step_read(
                                &mut framed,
                                &mut connection_activation,
                                &mut buf,
                            )
                            .await
                            .map_err(|e| session::custom_err!("read deactivation-reactivation sequence step", e))
                            .unwrap();

                            if written.size().is_some() {
                                framed
                                    .write_all(buf.filled())
                                    .await
                                    .map_err(|e| {
                                        session::custom_err!("write deactivation-reactivation sequence step", e)
                                    })
                                    .unwrap();
                            }

                            if let connector::connection_activation::ConnectionActivationState::Finalized {
                                desktop_size,
                                share_id,
                                input_flags: _,
                                enable_server_pointer,
                                pointer_software_rendering,
                                ..
                            } = connection_activation.connection_activation_state()
                            {
                                debug!(?desktop_size, "Deactivation-Reactivation Sequence completed");
                                // Update image size with the new desktop size.
                                // image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                                // Update the active stage with the new channel IDs and pointer settings.
                                stage.reactivate(
                                    connection_activation.io_channel_id(),
                                    connection_activation.user_channel_id(),
                                    share_id,
                                    enable_server_pointer,
                                    pointer_software_rendering,
                                );
                                break 'activation_seq;
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
            (stage, framed)
        },
    )
    .await
}

#[test]
fn test_reactivation_preserves_bulk_decompression_history() {
    let mut stage = ActiveStageBuilder {
        static_channels: StaticChannelSet::new(),
        user_channel_id: 1001,
        io_channel_id: 1003,
        message_channel_id: None,
        share_id: 1,
        compression_type: Some(PduCompressionType::K64),
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build();

    let mut image = DecodedImage::new(PixelFormat::RgbA32, 4, 4);
    let mut compressor = BulkCompressor::new(BulkCompressionType::Rdp5);
    let (first_frame, first_flags) = compressed_bitmap_fastpath_frame(&mut compressor);
    assert_ne!(first_flags & bulk_flags::PACKET_COMPRESSED, 0);

    stage
        .process(&mut image, pdu::Action::FastPath, &first_frame)
        .expect("compressed FastPath update before reactivation");

    stage.reactivate(1003, 1001, 2, false, false);

    let (second_frame, second_flags) = compressed_bitmap_fastpath_frame(&mut compressor);
    assert_ne!(second_flags & bulk_flags::PACKET_COMPRESSED, 0);
    assert_eq!(
        second_flags & (bulk_flags::PACKET_FLUSHED | bulk_flags::PACKET_AT_FRONT),
        0
    );

    let outputs = stage
        .process(&mut image, pdu::Action::FastPath, &second_frame)
        .expect("compressed FastPath update referencing pre-reactivation history");

    assert!(
        outputs
            .iter()
            .any(|output| matches!(output, ActiveStageOutput::GraphicsUpdate(_)))
    );
}

fn compressed_bitmap_fastpath_frame(compressor: &mut BulkCompressor) -> (Vec<u8>, u32) {
    let bitmap = BitmapUpdateData {
        rectangles: vec![BitmapData {
            rectangle: InclusiveRectangle {
                left: 0,
                top: 0,
                right: 3,
                bottom: 3,
            },
            width: 4,
            height: 4,
            bits_per_pixel: 32,
            compression_flags: Compression::empty(),
            compressed_data_header: None,
            bitmap_data: &[0x40; 64],
        }],
    };
    let bitmap_data = encode_vec(&bitmap).expect("encode bitmap update");

    let (compressed_size, flags) = compressor.compress(&bitmap_data).expect("compress bitmap update");

    let compressed_data = compressor.compressed_data(compressed_size);
    let update = FastPathUpdatePdu {
        fragmentation: Fragmentation::Single,
        update_code: UpdateCode::Bitmap,
        compression_flags: Some(CompressionFlags::from_bits_retain(
            u8::try_from(flags & !bulk_flags::COMPRESSION_TYPE_MASK).expect("compression flags fit in u8"),
        )),
        compression_type: Some(PduCompressionType::K64),
        data: compressed_data,
    };
    let header = FastPathHeader::new(EncryptionFlags::empty(), update.size());

    let mut frame = encode_vec(&header).expect("encode FastPath header");
    frame.extend(encode_vec(&update).expect("encode FastPath update"));
    (frame, flags)
}

#[tokio::test]
async fn test_echo_virtual_channel_end_to_end() {
    let payload = b"ironrdp echo e2e".to_vec();
    let echo_payload = payload.clone();

    client_server_with_connector(
        default_client_config(),
        None,
        |connector| connector.with_static_channel(DrdynvcClient::new().with_dynamic_channel(EchoClient::new())),
        move |mut stage, _activation_factory, mut framed, display_tx, echo_handle| async move {
            let _display_tx = display_tx;
            let mut image = DecodedImage::new(PixelFormat::RgbA32, DESKTOP_WIDTH, DESKTOP_HEIGHT);

            let deadline = Instant::now() + Duration::from_secs(5);
            let mut matched_measurement = None;

            while Instant::now() < deadline {
                echo_handle
                    .send_request(echo_payload.clone())
                    .expect("send echo request");

                for _ in 0..20 {
                    let measurements = echo_handle.take_measurements();
                    if let Some(measurement) = measurements.into_iter().find(|m| m.payload == echo_payload) {
                        matched_measurement = Some(measurement);
                        break;
                    }

                    let read_result = tokio::time::timeout(Duration::from_millis(150), framed.read_pdu()).await;
                    let Ok(Ok((action, frame))) = read_result else {
                        continue;
                    };

                    let outputs = stage.process(&mut image, action, &frame).expect("stage process");
                    for output in outputs {
                        if let ActiveStageOutput::ResponseFrame(frame) = output {
                            framed.write_all(&frame).await.expect("write response frame");
                        }
                    }
                }

                if matched_measurement.is_some() {
                    break;
                }
            }

            let measurement = matched_measurement.expect("echo RTT measurement was not produced");
            assert_eq!(measurement.payload, echo_payload);

            (stage, framed)
        },
    )
    .await
}

#[tokio::test]
async fn tls_validation_preserves_the_default_and_strict_is_explicit() {
    let cert_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-cert.pem");
    let key_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-key.pem");
    let identity = TlsIdentityCtx::init_from_paths(&cert_path, &key_path).expect("failed to init TLS identity");
    let acceptor = identity.make_acceptor().expect("failed to build TLS acceptor");
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind TLS test listener");
    let address = listener.local_addr().expect("TLS test listener address");

    let server = tokio::spawn(async move {
        for expected_success in [true, false, true] {
            let (stream, _) = listener.accept().await.expect("accept TLS test connection");
            let result = acceptor.accept(stream).await;
            assert_eq!(result.is_ok(), expected_success);
        }
    });

    let (tls_stream, _) = ironrdp_tls::upgrade(
        TcpStream::connect(address).await.expect("connect default TLS client"),
        "localhost",
    )
    .await
    .expect("default validation accepts the self-signed test certificate");
    drop(tls_stream);

    let strict_result = ironrdp_tls::upgrade_with_certificate_validation(
        TcpStream::connect(address).await.expect("connect strict TLS client"),
        "localhost",
        ironrdp_tls::CertificateValidation::Strict,
    )
    .await;
    assert!(
        strict_result.is_err(),
        "strict validation must reject the self-signed test certificate"
    );

    let callback_called = Arc::new(AtomicBool::new(false));
    let callback_called_for_callback = Arc::clone(&callback_called);
    let callback: ironrdp_tls::CertificateValidationCallback = Arc::new(move |certificate, reason| {
        callback_called_for_callback.store(true, Ordering::Relaxed);
        !certificate.is_empty() && !reason.is_empty()
    });
    let (tls_stream, _) = ironrdp_tls::upgrade_with_certificate_validation_callback(
        TcpStream::connect(address).await.expect("connect callback TLS client"),
        "localhost",
        callback,
    )
    .await
    .expect("TLS callback accepts the self-signed test certificate");
    drop(tls_stream);

    assert!(callback_called.load(Ordering::Relaxed));
    server.await.expect("TLS test server task");
}

#[tokio::test]
async fn rdpdr_static_drive_announces_and_completes_create_request() {
    let events = Arc::new(StdMutex::new(Vec::new()));
    let fixture_events = Arc::clone(&events);

    client_server_with_connector(
        default_client_config(),
        Some(Box::new(RdpdrFixtureFactory {
            events: Arc::clone(&fixture_events),
            create_path: r"\fixture-probe-file".to_owned(),
            read_requests: Vec::new(),
        })),
        |connector| {
            connector
                .with_static_channel(Rdpsnd::new(Box::new(NoopRdpsndBackend)))
                .with_static_channel(
                    Rdpdr::new(Box::new(NoopRdpdrBackend), "ironrdp-test".to_owned())
                        .with_drives(Some(vec![(RDPDR_TEST_DRIVE_ID, "test-drive".to_owned())])),
                )
        },
        move |mut stage, _activation_factory, mut framed, display_tx, _echo_handle| async move {
            let _display_tx = display_tx;
            let mut image = DecodedImage::new(PixelFormat::RgbA32, DESKTOP_WIDTH, DESKTOP_HEIGHT);
            let deadline = Instant::now() + Duration::from_secs(5);

            while Instant::now() < deadline {
                if fixture_events
                    .lock()
                    .expect("RDPDR fixture events lock")
                    .iter()
                    .any(|event| matches!(event, RdpdrFixtureEvent::CreateCompletion { .. }))
                {
                    break;
                }

                let read_result = tokio::time::timeout(Duration::from_millis(150), framed.read_pdu()).await;
                let Ok(Ok((action, frame))) = read_result else {
                    continue;
                };

                let outputs = stage
                    .process(&mut image, action, &frame)
                    .expect("process RDPDR fixture frame");
                for output in outputs {
                    match output {
                        ActiveStageOutput::ResponseFrame(frame) => {
                            framed.write_all(&frame).await.expect("write RDPDR fixture response");
                        }
                        other => panic!("unexpected RDPDR fixture output: {other:?}"),
                    }
                }
            }

            (stage, framed)
        },
    )
    .await;

    assert_eq!(
        *events.lock().expect("RDPDR fixture events lock"),
        vec![
            RdpdrFixtureEvent::ServerAnnounce,
            RdpdrFixtureEvent::ClientAnnounce,
            RdpdrFixtureEvent::ClientName,
            RdpdrFixtureEvent::ClientCapabilities,
            RdpdrFixtureEvent::ServerClientIdConfirm,
            RdpdrFixtureEvent::UserLoggedOn,
            RdpdrFixtureEvent::ClientDeviceListAnnounce,
            RdpdrFixtureEvent::DeviceAccepted,
            RdpdrFixtureEvent::CreateCompletion {
                status: u32::from(NtStatus::NOT_SUPPORTED),
                file_id: 0,
            },
        ]
    );
}

#[cfg(windows)]
#[tokio::test]
async fn rdpdr_windows_backend_creates_file_through_static_channel() {
    let temporary_directory = std::env::temp_dir().join(format!("ironrdp-rdpdr-e2e-{}", Uuid::new_v4()));
    std::fs::create_dir(&temporary_directory).expect("create RDPDR temporary directory");
    let temporary_file = temporary_directory.join("fixture-probe-file");
    let volume_root = volume_root(&temporary_directory);
    let remote_path = format!(
        r"\{}",
        temporary_file
            .strip_prefix(&volume_root)
            .expect("temporary file is beneath its volume root")
            .display()
    );

    let events = Arc::new(StdMutex::new(Vec::new()));
    let fixture_events = Arc::clone(&events);
    let drive = ironrdp_rdpdr_native::RedirectedDrive::new(RDPDR_TEST_DRIVE_ID, "test-drive", &volume_root, false)
        .expect("valid redirected drive");
    let factory =
        ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::new(vec![drive]).expect("unique redirected drive ID");
    let product = factory.build_rdpdr_backend().expect("open redirected volume root");
    let drives = product
        .initial_drives
        .into_iter()
        .map(|drive| (drive.device_id, drive.name))
        .collect();

    client_server_with_connector(
        default_client_config(),
        Some(Box::new(RdpdrFixtureFactory {
            events: Arc::clone(&fixture_events),
            create_path: remote_path,
            read_requests: Vec::new(),
        })),
        |connector| {
            connector
                .with_static_channel(Rdpsnd::new(Box::new(NoopRdpsndBackend)))
                .with_static_channel(Rdpdr::new(product.backend, "ironrdp-test".to_owned()).with_drives(Some(drives)))
        },
        move |mut stage, _activation_factory, mut framed, display_tx, _echo_handle| async move {
            let _display_tx = display_tx;
            let mut image = DecodedImage::new(PixelFormat::RgbA32, DESKTOP_WIDTH, DESKTOP_HEIGHT);
            let deadline = Instant::now() + Duration::from_secs(5);

            while Instant::now() < deadline {
                if fixture_events
                    .lock()
                    .expect("RDPDR fixture events lock")
                    .iter()
                    .any(|event| matches!(event, RdpdrFixtureEvent::CreateCompletion { .. }))
                {
                    break;
                }

                let read_result = tokio::time::timeout(Duration::from_millis(150), framed.read_pdu()).await;
                let Ok(Ok((action, frame))) = read_result else {
                    continue;
                };

                let outputs = stage
                    .process(&mut image, action, &frame)
                    .expect("process RDPDR fixture frame");
                for output in outputs {
                    match output {
                        ActiveStageOutput::ResponseFrame(frame) => {
                            framed.write_all(&frame).await.expect("write RDPDR fixture response");
                        }
                        other => panic!("unexpected RDPDR fixture output: {other:?}"),
                    }
                }
            }

            (stage, framed)
        },
    )
    .await;

    assert!(
        temporary_file.is_file(),
        "RDPDR create request did not create the temporary file"
    );
    assert!(
        events
            .lock()
            .expect("RDPDR fixture events lock")
            .iter()
            .any(|event| matches!(
                event,
                RdpdrFixtureEvent::CreateCompletion {
                    status,
                    file_id,
                } if *status == u32::from(NtStatus::SUCCESS) && *file_id != 0
            )),
        "native RDPDR backend did not return a successful create completion"
    );

    std::fs::remove_dir_all(&temporary_directory).expect("remove RDPDR temporary directory");
}

#[cfg(windows)]
#[tokio::test]
async fn rdpdr_windows_backend_preserves_large_reads_through_static_channel() {
    let temporary_directory = std::env::temp_dir().join(format!("ironrdp-rdpdr-e2e-{}", Uuid::new_v4()));
    std::fs::create_dir(&temporary_directory).expect("create RDPDR temporary directory");
    let temporary_file = temporary_directory.join("fixture-probe-file");
    std::fs::write(
        &temporary_file,
        vec![0x5A; usize::try_from(RDPDR_TEST_LARGE_READ_REQUEST_LENGTH).expect("read length fits in usize") + 1],
    )
    .expect("write RDPDR temporary file");
    let volume_root = volume_root(&temporary_directory);
    let remote_path = format!(
        r"\{}",
        temporary_file
            .strip_prefix(&volume_root)
            .expect("temporary file is beneath its volume root")
            .display()
    );

    let events = Arc::new(StdMutex::new(Vec::new()));
    let fixture_events = Arc::clone(&events);
    let drive = ironrdp_rdpdr_native::RedirectedDrive::new(RDPDR_TEST_DRIVE_ID, "test-drive", &volume_root, false)
        .expect("valid redirected drive");
    let factory =
        ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::new(vec![drive]).expect("unique redirected drive ID");
    let product = factory.build_rdpdr_backend().expect("open redirected volume root");
    let drives = product
        .initial_drives
        .into_iter()
        .map(|drive| (drive.device_id, drive.name))
        .collect();

    client_server_with_connector(
        default_client_config(),
        Some(Box::new(RdpdrFixtureFactory {
            events: Arc::clone(&fixture_events),
            create_path: remote_path,
            read_requests: vec![
                RdpdrFixtureReadRequest {
                    length: RDPDR_TEST_LARGE_READ_REQUEST_LENGTH,
                    offset: 0,
                },
                RdpdrFixtureReadRequest {
                    length: 1,
                    offset: u64::from(RDPDR_TEST_LARGE_READ_REQUEST_LENGTH),
                },
            ],
        })),
        |connector| {
            connector
                .with_static_channel(Rdpsnd::new(Box::new(NoopRdpsndBackend)))
                .with_static_channel(Rdpdr::new(product.backend, "ironrdp-test".to_owned()).with_drives(Some(drives)))
        },
        move |mut stage, _activation_factory, mut framed, display_tx, _echo_handle| async move {
            let _display_tx = display_tx;
            let mut image = DecodedImage::new(PixelFormat::RgbA32, DESKTOP_WIDTH, DESKTOP_HEIGHT);
            let deadline = Instant::now() + Duration::from_secs(5);

            while Instant::now() < deadline {
                if fixture_events
                    .lock()
                    .expect("RDPDR fixture events lock")
                    .iter()
                    .filter(|event| matches!(event, RdpdrFixtureEvent::ReadCompletion { .. }))
                    .count()
                    == 2
                {
                    break;
                }

                let read_result = tokio::time::timeout(Duration::from_millis(150), framed.read_pdu()).await;
                let Ok(Ok((action, frame))) = read_result else {
                    continue;
                };

                let outputs = stage
                    .process(&mut image, action, &frame)
                    .expect("process RDPDR fixture frame");
                for output in outputs {
                    match output {
                        ActiveStageOutput::ResponseFrame(frame) => {
                            framed.write_all(&frame).await.expect("write RDPDR fixture response");
                        }
                        other => panic!("unexpected RDPDR fixture output: {other:?}"),
                    }
                }
            }

            (stage, framed)
        },
    )
    .await;

    assert_eq!(
        events
            .lock()
            .expect("RDPDR fixture events lock")
            .iter()
            .filter_map(|event| match event {
                RdpdrFixtureEvent::ReadCompletion {
                    completion_id,
                    status,
                    length,
                } => Some((*completion_id, *status, *length)),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            (
                RDPDR_TEST_READ_COMPLETION_ID,
                u32::from(NtStatus::SUCCESS),
                RDPDR_TEST_LARGE_READ_REQUEST_LENGTH,
            ),
            (RDPDR_TEST_SECOND_READ_COMPLETION_ID, u32::from(NtStatus::SUCCESS), 1,),
        ],
        "native RDPDR backend did not preserve sequential large read responses"
    );

    std::fs::remove_dir_all(&temporary_directory).expect("remove RDPDR temporary directory");
}

#[cfg(windows)]
fn volume_root(path: &Path) -> PathBuf {
    let mut components = path.components();
    let prefix = components
        .next()
        .expect("Windows temporary directory has a volume prefix");
    let root = components.next().expect("Windows temporary directory is absolute");
    PathBuf::from(prefix.as_os_str()).join(root.as_os_str())
}

const RDPDR_TEST_CLIENT_ID: u32 = 0x45_52_44_50;
const RDPDR_TEST_DRIVE_ID: u32 = 0x0000_1001;
const RDPDR_TEST_CREATE_COMPLETION_ID: u32 = 0x0000_2001;
const RDPDR_TEST_READ_COMPLETION_ID: u32 = 0x0000_2002;
const RDPDR_TEST_SECOND_READ_COMPLETION_ID: u32 = 0x0000_2003;
const RDPDR_TEST_LARGE_READ_REQUEST_LENGTH: u32 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum RdpdrFixtureEvent {
    ServerAnnounce,
    ClientAnnounce,
    ClientName,
    ClientCapabilities,
    ServerClientIdConfirm,
    UserLoggedOn,
    ClientDeviceListAnnounce,
    DeviceAccepted,
    CreateCompletion {
        status: u32,
        file_id: u32,
    },
    ReadCompletion {
        completion_id: u32,
        status: u32,
        length: u32,
    },
}

#[derive(Clone, Copy, Debug)]
struct RdpdrFixtureReadRequest {
    length: u32,
    offset: u64,
}

#[derive(Debug)]
struct RdpdrFixtureFactory {
    events: Arc<StdMutex<Vec<RdpdrFixtureEvent>>>,
    create_path: String,
    read_requests: Vec<RdpdrFixtureReadRequest>,
}

impl server::StaticChannelFactory for RdpdrFixtureFactory {
    fn attach(&self, acceptor: &mut ironrdp::acceptor::Acceptor) {
        acceptor.attach_static_channel(RdpdrFixture {
            events: Arc::clone(&self.events),
            create_path: self.create_path.clone(),
            read_requests: self.read_requests.clone(),
            next_read_request: 0,
            active_file_id: None,
            stage: RdpdrFixtureStage::ClientAnnounce,
        });
    }
}

#[derive(Debug)]
struct RdpdrFixture {
    events: Arc<StdMutex<Vec<RdpdrFixtureEvent>>>,
    create_path: String,
    read_requests: Vec<RdpdrFixtureReadRequest>,
    next_read_request: usize,
    active_file_id: Option<u32>,
    stage: RdpdrFixtureStage,
}

#[derive(Clone, Copy, Debug)]
enum RdpdrFixtureStage {
    ClientAnnounce,
    ClientName,
    ClientCapabilities,
    ClientDeviceList,
    CreateCompletion,
    ReadCompletion,
}

impl ironrdp::core::AsAny for RdpdrFixture {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl RdpdrFixture {
    fn record(&self, event: RdpdrFixtureEvent) {
        self.events.lock().expect("RDPDR fixture events lock").push(event);
    }

    fn server_capabilities() -> RdpdrPdu {
        RdpdrPdu::CoreCapability(CoreCapability {
            capabilities: {
                let mut capabilities = Capabilities::new();
                capabilities.add_drive();
                capabilities.clone_inner()
            },
            kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
        })
    }

    fn server_client_id_confirm() -> RdpdrPdu {
        RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR_13,
            client_id: RDPDR_TEST_CLIENT_ID,
            kind: VersionAndIdPduKind::ServerClientIdConfirm,
        })
    }

    fn create_request(&self) -> Vec<u8> {
        let request = DeviceIoRequest {
            device_id: RDPDR_TEST_DRIVE_ID,
            file_id: 0,
            completion_id: RDPDR_TEST_CREATE_COMPLETION_ID,
            major_function: MajorFunction::Create,
            minor_function: MinorFunction::from(0),
        };
        let mut pdu = encode_vec(&RdpdrPdu::DeviceIoRequest(request)).expect("encode RDPDR create header");
        let mut encoded_path = Vec::with_capacity((self.create_path.len() + 1) * 2);
        for code_unit in self.create_path.encode_utf16().chain(core::iter::once(0)) {
            encoded_path.extend_from_slice(&code_unit.to_le_bytes());
        }

        pdu.extend_from_slice(&0xC000_0000u32.to_le_bytes()); // GENERIC_READ
        pdu.extend_from_slice(&0u64.to_le_bytes()); // AllocationSize
        pdu.extend_from_slice(&0x0000_0080u32.to_le_bytes()); // FILE_ATTRIBUTE_NORMAL
        pdu.extend_from_slice(&0x0000_0003u32.to_le_bytes()); // FILE_SHARE_READ | FILE_SHARE_WRITE
        pdu.extend_from_slice(&0x0000_0003u32.to_le_bytes()); // FILE_OPEN_IF
        pdu.extend_from_slice(&0x0000_0040u32.to_le_bytes()); // FILE_NON_DIRECTORY_FILE
        pdu.extend_from_slice(
            &u32::try_from(encoded_path.len())
                .expect("test create path fits in u32")
                .to_le_bytes(),
        );
        pdu.extend_from_slice(&encoded_path);
        pdu
    }

    fn next_read_request(&mut self, file_id: u32) -> Option<Vec<u8>> {
        let request = *self.read_requests.get(self.next_read_request)?;
        let completion_id = RDPDR_TEST_READ_COMPLETION_ID
            .checked_add(u32::try_from(self.next_read_request).expect("read request index fits in u32"))
            .expect("read completion ID does not overflow");
        self.next_read_request += 1;
        let device_io_request = DeviceIoRequest {
            device_id: RDPDR_TEST_DRIVE_ID,
            file_id,
            completion_id,
            major_function: MajorFunction::Read,
            minor_function: MinorFunction::from(0),
        };
        let mut pdu =
            encode_vec(&RdpdrPdu::DeviceIoRequest(device_io_request)).expect("encode RDPDR read request header");
        pdu.extend_from_slice(&request.length.to_le_bytes());
        pdu.extend_from_slice(&request.offset.to_le_bytes());
        pdu.resize(pdu.len() + 20, 0); // Padding
        Some(pdu)
    }
}

impl SvcProcessor for RdpdrFixture {
    fn channel_name(&self) -> gcc::ChannelName {
        Rdpdr::NAME
    }

    fn start(&mut self) -> pdu::PduResult<Vec<SvcMessage>> {
        self.record(RdpdrFixtureEvent::ServerAnnounce);
        Ok(vec![SvcMessage::from(RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR_13,
            client_id: RDPDR_TEST_CLIENT_ID,
            kind: VersionAndIdPduKind::ServerAnnounceRequest,
        }))])
    }

    fn process(&mut self, payload: &[u8]) -> pdu::PduResult<Vec<SvcMessage>> {
        let mut cursor = ReadCursor::new(payload);
        let header = SharedHeader::decode(&mut cursor).map_err(|error| pdu::decode_err!(error))?;

        match (self.stage, header.packet_id) {
            (RdpdrFixtureStage::ClientAnnounce, PacketId::CoreClientidConfirm) => {
                self.record(RdpdrFixtureEvent::ClientAnnounce);
                self.stage = RdpdrFixtureStage::ClientName;
                Ok(Vec::new())
            }
            (RdpdrFixtureStage::ClientName, PacketId::CoreClientName) => {
                self.record(RdpdrFixtureEvent::ClientName);
                self.stage = RdpdrFixtureStage::ClientCapabilities;
                Ok(vec![SvcMessage::from(Self::server_capabilities())])
            }
            (RdpdrFixtureStage::ClientCapabilities, PacketId::CoreClientCapability) => {
                self.record(RdpdrFixtureEvent::ClientCapabilities);
                self.record(RdpdrFixtureEvent::ServerClientIdConfirm);
                self.record(RdpdrFixtureEvent::UserLoggedOn);
                self.stage = RdpdrFixtureStage::ClientDeviceList;
                Ok(vec![
                    SvcMessage::from(Self::server_client_id_confirm()),
                    SvcMessage::from(RdpdrPdu::UserLoggedon),
                ])
            }
            (RdpdrFixtureStage::ClientDeviceList, PacketId::CoreDevicelistAnnounce) => {
                self.record(RdpdrFixtureEvent::ClientDeviceListAnnounce);
                self.record(RdpdrFixtureEvent::DeviceAccepted);
                self.stage = RdpdrFixtureStage::CreateCompletion;
                let device_reply = RdpdrPdu::ServerDeviceAnnounceResponse(ServerDeviceAnnounceResponse {
                    device_id: RDPDR_TEST_DRIVE_ID,
                    result_code: NtStatus::SUCCESS,
                });

                Ok(vec![
                    SvcMessage::from(device_reply),
                    SvcMessage::from(self.create_request()),
                ])
            }
            (RdpdrFixtureStage::CreateCompletion, PacketId::CoreDeviceIoCompletion) => {
                let response = DeviceIoResponse::decode(&mut cursor).map_err(|error| pdu::decode_err!(error))?;
                assert_eq!(response.device_id, RDPDR_TEST_DRIVE_ID);
                assert_eq!(response.completion_id, RDPDR_TEST_CREATE_COMPLETION_ID);
                let file_id = cursor.read_u32();
                let _information = cursor.read_u8();
                self.record(RdpdrFixtureEvent::CreateCompletion {
                    status: response.io_status.into(),
                    file_id,
                });
                self.active_file_id = Some(file_id);
                let Some(read_request) = self.next_read_request(file_id) else {
                    return Ok(Vec::new());
                };

                self.stage = RdpdrFixtureStage::ReadCompletion;
                Ok(vec![SvcMessage::from(read_request)])
            }
            (RdpdrFixtureStage::ReadCompletion, PacketId::CoreDeviceIoCompletion) => {
                let response = DeviceIoResponse::decode(&mut cursor).map_err(|error| pdu::decode_err!(error))?;
                assert_eq!(response.device_id, RDPDR_TEST_DRIVE_ID);
                let length = cursor.read_u32();
                assert_eq!(
                    usize::try_from(length).expect("RDPDR read response length fits in usize"),
                    cursor.len()
                );
                self.record(RdpdrFixtureEvent::ReadCompletion {
                    completion_id: response.completion_id,
                    status: response.io_status.into(),
                    length,
                });
                let file_id = self
                    .active_file_id
                    .expect("read completion follows a create completion");
                Ok(self
                    .next_read_request(file_id)
                    .map_or_else(Vec::new, |read_request| vec![SvcMessage::from(read_request)]))
            }
            _ => Err(pdu::pdu_other_err!(
                "RDPDR fixture",
                "received an unexpected client PDU"
            )),
        }
    }
}

impl SvcServerProcessor for RdpdrFixture {}

type DisplayUpdatesRx = Arc<Mutex<UnboundedReceiver<DisplayUpdate>>>;

struct TestDisplayUpdates {
    rx: DisplayUpdatesRx,
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for TestDisplayUpdates {
    async fn next_update(&mut self) -> Result<Option<DisplayUpdate>> {
        let mut rx = self.rx.lock().await;

        Ok(rx.recv().await)
    }
}

struct TestDisplay {
    rx: DisplayUpdatesRx,
}

#[async_trait::async_trait]
impl RdpServerDisplay for TestDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: DESKTOP_WIDTH,
            height: DESKTOP_HEIGHT,
        }
    }

    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(TestDisplayUpdates {
            rx: Arc::clone(&self.rx),
        }))
    }
}

struct TestInputHandler;
impl RdpServerInputHandler for TestInputHandler {
    fn keyboard(&mut self, _: KeyboardEvent) {}
    fn mouse(&mut self, _: MouseEvent) {}
}

async fn client_server<F, Fut>(client_config: connector::Config, clientfn: F)
where
    F: FnOnce(
            ActiveStage,
            connector::connection_activation::ConnectionActivationFactory,
            Framed<TokioStream<TlsStream<TcpStream>>>,
            UnboundedSender<DisplayUpdate>,
        ) -> Fut
        + 'static,
    Fut: Future<Output = (ActiveStage, Framed<TokioStream<TlsStream<TcpStream>>>)>,
{
    client_server_with_connector(
        client_config,
        None,
        |connector| connector,
        move |stage, connection_activation, framed, display_tx, _echo_handle| {
            clientfn(stage, connection_activation, framed, display_tx)
        },
    )
    .await;
}

async fn client_server_with_connector<F, Fut, C>(
    client_config: connector::Config,
    static_channel_factory: Option<Box<dyn server::StaticChannelFactory>>,
    connector_factory: C,
    clientfn: F,
) where
    F: FnOnce(
            ActiveStage,
            connector::connection_activation::ConnectionActivationFactory,
            Framed<TokioStream<TlsStream<TcpStream>>>,
            UnboundedSender<DisplayUpdate>,
            server::EchoServerHandle,
        ) -> Fut
        + 'static,
    Fut: Future<Output = (ActiveStage, Framed<TokioStream<TlsStream<TcpStream>>>)>,
    C: FnOnce(connector::ClientConnector) -> connector::ClientConnector + 'static,
{
    // FIXME(@CBenoit): If this is really necessary, we may consider a non-global way of registering the subscriber; otherwise it’s unnecessary to register that.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let cert_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-cert.pem");
    let key_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-key.pem");
    let identity = TlsIdentityCtx::init_from_paths(&cert_path, &key_path).expect("failed to init TLS identity");
    let acceptor = identity.make_acceptor().expect("failed to build TLS acceptor");

    let (display_tx, display_rx) = mpsc::unbounded_channel();
    let server_builder = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_tls(acceptor)
        .with_input_handler(TestInputHandler)
        .with_display_handler(TestDisplay {
            rx: Arc::new(Mutex::new(display_rx)),
        });
    let server_builder = if let Some(factory) = static_channel_factory {
        server_builder.with_static_channel_factory(factory)
    } else {
        server_builder
    };
    let mut server = server_builder.build();
    server.set_credentials(Some(server::Credentials {
        username: USERNAME.into(),
        password: PASSWORD.into(),
        domain: None,
    }));
    let ev = server.event_sender().clone();
    let echo_handle = server.echo_handle().clone();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let server = tokio::task::spawn_local(async move {
                server.run().await.unwrap();
            });

            let client = tokio::task::spawn_local(async move {
                let (tx, rx) = oneshot::channel();
                ev.send(ServerEvent::GetLocalAddr(tx)).unwrap();
                let server_addr = rx.await.unwrap().unwrap();
                let tcp_stream = TcpStream::connect(server_addr).await.expect("TCP connect");
                let client_addr = tcp_stream.local_addr().expect("local_addr");
                let mut framed = ironrdp_tokio::TokioFramed::new(tcp_stream);
                let connector = connector::ClientConnector::new(client_config, client_addr);
                let mut connector = connector_factory(connector);
                let should_upgrade = ironrdp_async::connect_begin(&mut framed, &mut connector)
                    .await
                    .expect("begin connection");
                let initial_stream = framed.into_inner_no_leftover();
                let (upgraded_stream, tls_cert) = ironrdp_tls::upgrade_with_certificate_validation(
                    initial_stream,
                    "localhost",
                    ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate,
                )
                .await
                .expect("TLS upgrade");
                let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
                let mut upgraded_framed = ironrdp_tokio::TokioFramed::new(upgraded_stream);
                let server_public_key =
                    ironrdp_tls::extract_tls_server_public_key(&tls_cert).expect("extract server public key");
                let connection_result = ironrdp_async::connect_finalize(
                    upgraded,
                    connector,
                    &mut upgraded_framed,
                    &mut ironrdp_tokio::reqwest::ReqwestNetworkClient::new(),
                    "localhost".into(),
                    server_public_key.to_owned(),
                    None,
                )
                .await
                .expect("finalize connection");

                // Retain the connection activation factory so the client closure can drive its own
                // Deactivation-Reactivation Sequence.
                let activation_factory = connection_result.activation_factory;
                let active_stage = ActiveStageBuilder {
                    static_channels: connection_result.static_channels,
                    user_channel_id: connection_result.user_channel_id,
                    io_channel_id: connection_result.io_channel_id,
                    message_channel_id: connection_result.message_channel_id,
                    share_id: connection_result.share_id,
                    compression_type: connection_result.compression_type,
                    enable_server_pointer: connection_result.enable_server_pointer,
                    pointer_software_rendering: connection_result.pointer_software_rendering,
                }
                .build();
                let (active_stage, mut upgraded_framed) = clientfn(
                    active_stage,
                    activation_factory,
                    upgraded_framed,
                    display_tx,
                    echo_handle,
                )
                .await;
                let outputs = active_stage.graceful_shutdown().expect("shutdown");
                for out in outputs {
                    match out {
                        ActiveStageOutput::ResponseFrame(frame) => {
                            upgraded_framed.write_all(&frame).await.expect("write frame");
                        }
                        _ => unimplemented!(),
                    }
                }

                // server should probably send TLS close_notify
                while let Ok(pdu) = upgraded_framed.read_pdu().await {
                    debug!(?pdu);
                }
                ev.send(ServerEvent::Quit("bye".into())).unwrap();
            });

            tokio::try_join!(server, client).expect("join");
        })
        .await;
}

fn default_client_config() -> connector::Config {
    connector::Config {
        desktop_size: DesktopSize {
            width: DESKTOP_WIDTH,
            height: DESKTOP_HEIGHT,
        },
        desktop_scale_factor: 0, // Default to 0 per FreeRDP
        enable_tls: true,
        enable_credssp: true,
        enable_standard_rdp_security: false,
        credentials: connector::Credentials::UsernamePassword {
            username: USERNAME.into(),
            password: PASSWORD.into(),
        },
        domain: None,
        client_build: semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .map(|version| version.major * 100 + version.minor * 10 + version.patch)
            .unwrap_or(0)
            .try_into()
            .unwrap(),
        client_name: "ironrdp".into(),
        keyboard_type: gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        connection_type: gcc::ConnectionType::Lan,
        ime_file_name: "".into(),
        bitmap: None,
        dig_product_id: "".into(),
        // NOTE: hardcode this value like in freerdp
        // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
        client_dir: "C:\\Windows\\System32\\mstscax.dll".into(),
        #[cfg(windows)]
        platform: MajorPlatformType::WINDOWS,
        #[cfg(target_os = "macos")]
        platform: MajorPlatformType::MACINTOSH,
        #[cfg(target_os = "ios")]
        platform: MajorPlatformType::IOS,
        #[cfg(target_os = "linux")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "android")]
        platform: MajorPlatformType::ANDROID,
        #[cfg(target_os = "freebsd")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "dragonfly")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "openbsd")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "netbsd")]
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: true,
        license_cache: None,
        compression_type: None,
        enable_server_pointer: true,
        pointer_software_rendering: true,
        multitransport_flags: None,
        performance_flags: Default::default(),
        timezone_info: Default::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}
