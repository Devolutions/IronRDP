// FIXME: tests in this module can probably be rewritten to be much shorter using the ironrdp-client crate.

use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use ironrdp::connector;
use ironrdp::core::{Encode as _, encode_vec, impl_as_any};
use ironrdp::dvc::DrdynvcClient;
use ironrdp::echo::client::EchoClient;
use ironrdp::pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp::pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::CompressionType as PduCompressionType;
use ironrdp::pdu::rdp::headers::CompressionFlags;
use ironrdp::pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode};
use ironrdp::pdu::rdp::session_info::ServerAutoReconnect;
use ironrdp::pdu::{self, gcc};
use ironrdp::server::{
    self, Acceptor, DesktopSize, DisplayContext, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RdpServer,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, ServerEvent, ServerResult, StaticChannelFactory,
    TlsIdentityCtx,
};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{self, ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp::svc::{StaticChannelSet, SvcMessage, SvcProcessor, SvcServerProcessor};
use ironrdp_async::{Framed, FramedWrite as _};
use ironrdp_bulk::{BulkCompressor, CompressionType as BulkCompressionType, flags as bulk_flags};
use ironrdp_dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor};
use ironrdp_egfx::pdu::{CapabilitiesAdvertisePdu, CapabilitiesV8Flags, CapabilitySet, GfxPdu};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    Capabilities, CoreCapability, CoreCapabilityKind, DeviceCreateResponse, DeviceIoRequest, DeviceIoResponse,
    MajorFunction, MinorFunction, NtStatus, ServerDeviceAnnounceResponse, ServerDriveIoRequest, VERSION_MAJOR,
    VERSION_MINOR_12, VersionAndIdPdu, VersionAndIdPduKind,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::{Rdpdr, RdpdrBackend, RdpdrBackendFactory, RdpdrBackendProduct, RdpdrDrive};
use ironrdp_server::{GfxServerFactory, ServerEventSender};
use ironrdp_testsuite_extra as _;
use ironrdp_tls::TlsStream;
use ironrdp_tokio::TokioStream;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, oneshot};
use tracing::debug;

const DESKTOP_WIDTH: u16 = 1024;
const DESKTOP_HEIGHT: u16 = 768;
const USERNAME: &str = "";
const PASSWORD: &str = "";
const RDPDR_DEVICE_ID: u32 = 1;
const RDPDR_CLIENT_ID: u32 = 0x1234;
const RDPDR_COMPLETION_ID: u32 = 1;
#[cfg(windows)]
const RDPDR_READ_LENGTH: u32 = 32 * 1024;

#[tokio::test]
async fn test_client_server() {
    client_server(
        default_client_config(),
        |stage, _activation_factory, framed, _display_tx| async { (stage, framed) },
    )
    .await
}

/// Configuring UDP multitransport on the server must not disturb a client
/// that never advertises support for it: `set_multitransport_offer` gates on
/// the client's own GCC `MultiTransportChannelData` reciprocating, so with
/// `multitransport_flags: None` (the default) the acceptor never sends the
/// Initiate Multitransport Request and the connection proceeds exactly as it
/// does with no UDP transport configured at all.
#[tokio::test]
async fn test_client_server_with_udp_transport_configured_but_unused() {
    client_server_with_connector(
        default_client_config(),
        Vec::new(),
        Some(([127, 0, 0, 1], 0).into()),
        |connector| connector,
        |stage, _activation_factory, framed, _display_tx, _echo_handle| async { (stage, framed) },
    )
    .await;
}

/// Advertising the Graphics Pipeline early-capability bit must not disturb connection establishment.
///
/// The core testsuite separately decodes the emitted Connect Initial PDU and verifies the capability flag.
#[tokio::test]
async fn test_client_server_advertising_dyn_vc_gfx_protocol() {
    client_server(
        connector::Config {
            support_dyn_vc_gfx_protocol: true,
            ..default_client_config()
        },
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
                        run_reactivation(&mut stage, &activation_factory, &mut framed).await;
                    }
                    _ => unreachable!(),
                }
            }
            (stage, framed)
        },
    )
    .await
}

/// Executes the client side of the Deactivation-Reactivation Sequence, after the server's
/// Deactivate All PDU has been received:
/// <https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432>
async fn run_reactivation(
    stage: &mut ActiveStage,
    activation_factory: &connector::connection_activation::ConnectionActivationFactory,
    framed: &mut Framed<TokioStream<TlsStream<TcpStream>>>,
) {
    debug!("Received Server Deactivate All PDU, executing Deactivation-Reactivation Sequence");
    let mut connection_activation = activation_factory.create();
    let mut buf = pdu::WriteBuf::new();
    loop {
        let written = ironrdp_async::single_sequence_step_read(framed, &mut connection_activation, &mut buf)
            .await
            .map_err(|e| session::custom_err!("read deactivation-reactivation sequence step", e))
            .unwrap();

        if written.size().is_some() {
            framed
                .write_all(buf.filled())
                .await
                .map_err(|e| session::custom_err!("write deactivation-reactivation sequence step", e))
                .unwrap();
        }

        if let connector::connection_activation::ConnectionActivationState::Finalized {
            desktop_size,
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
            static_channel_chunk_size,
            ..
        } = connection_activation.connection_activation_state()
        {
            debug!(?desktop_size, "Deactivation-Reactivation Sequence completed");
            // Update the active stage with the new channel IDs and pointer settings.
            assert!(stage.reactivate(
                connection_activation.io_channel_id(),
                connection_activation.user_channel_id(),
                share_id,
                enable_server_pointer,
                pointer_software_rendering,
                static_channel_chunk_size,
            ));
            return;
        }
    }
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

    assert!(stage.reactivate(1003, 1001, 2, false, false, 1600));

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

/// Sends echo requests and serves the client session until the server reports
/// the round trip for `payload`, or five seconds pass.
async fn drive_echo_round_trip(
    stage: &mut ActiveStage,
    framed: &mut Framed<TokioStream<TlsStream<TcpStream>>>,
    echo_handle: &server::EchoServerHandle,
    payload: &[u8],
) -> Option<server::EchoRoundTripMeasurement> {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, DESKTOP_WIDTH, DESKTOP_HEIGHT);
    let deadline = Instant::now() + Duration::from_secs(5);

    while Instant::now() < deadline {
        echo_handle.send_request(payload.to_vec()).expect("send echo request");

        for _ in 0..20 {
            if let Some(measurement) = echo_handle
                .take_measurements()
                .into_iter()
                .find(|measurement| measurement.payload == payload)
            {
                return Some(measurement);
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
    }

    None
}

#[tokio::test]
async fn test_echo_virtual_channel_end_to_end() {
    let payload = b"ironrdp echo e2e".to_vec();
    let echo_payload = payload.clone();

    client_server_with_connector(
        default_client_config(),
        Vec::new(),
        None,
        |connector| connector.with_static_channel(DrdynvcClient::new().with_dynamic_channel(EchoClient::new())),
        move |mut stage, _activation_factory, mut framed, display_tx, echo_handle| async move {
            let _display_tx = display_tx;

            let measurement = drive_echo_round_trip(&mut stage, &mut framed, &echo_handle, &echo_payload)
                .await
                .expect("echo RTT measurement was not produced");
            assert_eq!(measurement.payload, echo_payload);

            (stage, framed)
        },
    )
    .await
}

/// A previous session's queued events must not reach the next connection.
///
/// A `Disconnect` left over from the last session would otherwise end this one
/// as soon as it activates, so the echo round trip never completes.
#[tokio::test]
async fn a_new_connection_starts_without_the_previous_sessions_events() {
    let payload = b"ironrdp stale events e2e".to_vec();
    let echo_payload = payload.clone();
    let stale_disconnect = ServerEvent::Disconnect(ErrorInfo::ProtocolIndependentCode(
        ProtocolIndependentCode::DisconnectedByOtherconnection,
    ));

    client_server_impl(
        Some(vec![stale_disconnect]),
        default_client_config(),
        Vec::new(),
        None,
        |connector| connector.with_static_channel(DrdynvcClient::new().with_dynamic_channel(EchoClient::new())),
        move |mut stage, _activation_factory, mut framed, display_tx, echo_handle| async move {
            let _display_tx = display_tx;

            assert!(
                drive_echo_round_trip(&mut stage, &mut framed, &echo_handle, &echo_payload)
                    .await
                    .is_some(),
                "the connection did not survive a Disconnect event queued before it started"
            );

            (stage, framed)
        },
    )
    .await
}

#[tokio::test]
async fn rdpdr_static_channel_announces_a_drive_and_completes_an_unsupported_create() {
    let fixture = RdpdrFixtureFactory::new(RdpdrFixtureOperation::UnsupportedCreate);
    let fixture_state = fixture.state();

    client_server_with_connector(
        default_client_config(),
        vec![Box::new(fixture)],
        None,
        |connector| connector.with_static_channel(test_rdpdr_channel()),
        move |stage, _activation_factory, framed, display_tx, _echo_handle| {
            drive_rdpdr_until_complete(stage, framed, display_tx, fixture_state)
        },
    )
    .await;
}

#[cfg(windows)]
#[tokio::test]
async fn rdpdr_static_channel_creates_a_file_with_the_windows_backend() {
    use std::fs;

    let root = test_directory("create");
    fs::create_dir_all(&root).expect("create redirected-drive root");
    let factory = ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::new(
        ironrdp_rdpdr_native::RedirectedDrive::new(RDPDR_DEVICE_ID, "test", volume_root(&root), false)
            .expect("valid redirected drive"),
    );
    let fixture = RdpdrFixtureFactory::new(RdpdrFixtureOperation::CreateFile {
        path: volume_relative_path(&root, "created.txt"),
    });
    let fixture_state = fixture.state();
    let fixture_state_for_client = Arc::clone(&fixture_state);

    client_server_with_connector(
        default_client_config(),
        vec![Box::new(fixture)],
        None,
        move |connector| connector.with_static_channel(rdpdr_channel(&factory)),
        move |stage, _activation_factory, framed, display_tx, _echo_handle| {
            drive_rdpdr_until_complete(stage, framed, display_tx, fixture_state_for_client)
        },
    )
    .await;

    assert!(root.join("created.txt").is_file());
    fs::remove_dir_all(root).expect("remove redirected-drive root");
}

#[cfg(windows)]
#[tokio::test]
async fn rdpdr_static_channel_preserves_large_read_response_lengths() {
    use std::fs;

    let root = test_directory("read");
    fs::create_dir_all(&root).expect("create redirected-drive root");
    fs::write(
        root.join("fixture.bin"),
        vec![0xA5; usize::try_from(RDPDR_READ_LENGTH * 2).expect("read length fits usize")],
    )
    .expect("write redirected file");
    let factory = ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::new(
        ironrdp_rdpdr_native::RedirectedDrive::new(RDPDR_DEVICE_ID, "test", volume_root(&root), false)
            .expect("valid redirected drive"),
    );
    let fixture = RdpdrFixtureFactory::new(RdpdrFixtureOperation::ReadFile {
        path: volume_relative_path(&root, "fixture.bin"),
        response_lengths: Vec::new(),
    });
    let fixture_state = fixture.state();
    let fixture_state_for_client = Arc::clone(&fixture_state);

    client_server_with_connector(
        default_client_config(),
        vec![Box::new(fixture)],
        None,
        move |connector| connector.with_static_channel(rdpdr_channel(&factory)),
        move |stage, _activation_factory, framed, display_tx, _echo_handle| {
            drive_rdpdr_until_complete(stage, framed, display_tx, fixture_state_for_client)
        },
    )
    .await;

    let state = fixture_state.lock().expect("fixture state");
    let RdpdrFixtureOperation::ReadFile { response_lengths, .. } = &state.operation else {
        unreachable!("read fixture remains configured for reads");
    };
    assert_eq!(response_lengths, &[RDPDR_READ_LENGTH, RDPDR_READ_LENGTH]);
    drop(state);
    fs::remove_dir_all(root).expect("remove redirected-drive root");
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
    let callback: ironrdp_tls::CertificateValidationCallback = Arc::new(move |certificate, server_name, reason| {
        callback_called_for_callback.store(true, Ordering::Relaxed);
        !certificate.is_empty() && server_name == "localhost" && !reason.is_empty()
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

/// The display backend gets the handles the server writes, the same ones again
/// after a Deactivation-Reactivation Sequence, and a new connection gets its
/// own, back at their initial values whatever the previous connection left in
/// its handles.
#[tokio::test]
async fn each_connection_gets_its_own_display_context() {
    let cert_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-cert.pem");
    let key_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-key.pem");
    let identity = TlsIdentityCtx::init_from_paths(&cert_path, &key_path).expect("failed to init TLS identity");
    let acceptor = identity.make_acceptor().expect("failed to build TLS acceptor");

    let (ctx_tx, mut ctx_rx) = mpsc::unbounded_channel();
    let (display_tx, display_rx) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_tls(acceptor)
        .with_input_handler(TestInputHandler)
        .with_display_handler(RecordingDisplay {
            contexts: ctx_tx,
            updates: Arc::new(Mutex::new(display_rx)),
        })
        .build();
    server.set_credentials(Some(server::Credentials {
        username: USERNAME.into(),
        password: PASSWORD.into(),
        domain: None,
    }));
    server.enable_autodetect();
    let ev = server.event_sender().clone();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let server = tokio::task::spawn_local(async move {
                server.run().await.unwrap();
            });

            let client = tokio::task::spawn_local(async move {
                let server_addr = local_addr_of(&ev).await;

                let (mut stage, activation_factory, mut framed) = connect(server_addr).await;
                let first = ctx_rx.recv().await.expect("first connection's display context");

                // The client answers an RTT probe on its own, and the server
                // records the result on the handles it gave the display.
                ev.send(ServerEvent::AutoDetectRttRequest).unwrap();
                let mut image = DecodedImage::new(PixelFormat::RgbA32, DESKTOP_WIDTH, DESKTOP_HEIGHT);
                'probe: loop {
                    let (action, payload) = framed.read_pdu().await.expect("valid PDU");
                    for out in stage.process(&mut image, action, &payload).expect("stage process") {
                        if let ActiveStageOutput::ResponseFrame(frame) = out {
                            framed.write_all(&frame).await.expect("write RTT response");
                            break 'probe;
                        }
                    }
                }
                tokio::time::timeout(Duration::from_secs(5), async {
                    while first.autodetect.rtt.load(Ordering::Relaxed) == u32::MAX {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("the RTT reaches the display's handle");

                // A resize deactivates and reactivates the same connection,
                // and the display is asked for its updates again.
                display_tx
                    .send(DisplayUpdate::Resize(DesktopSize {
                        width: 2048,
                        height: 2048,
                    }))
                    .unwrap();
                'deactivate: loop {
                    let (action, payload) = framed.read_pdu().await.expect("valid PDU");
                    for out in stage.process(&mut image, action, &payload).expect("stage process") {
                        if matches!(out, ActiveStageOutput::DeactivateAll) {
                            break 'deactivate;
                        }
                    }
                }
                let mut connection_activation = activation_factory.create();
                let mut buf = pdu::WriteBuf::new();
                loop {
                    let written =
                        ironrdp_async::single_sequence_step_read(&mut framed, &mut connection_activation, &mut buf)
                            .await
                            .expect("read deactivation-reactivation sequence step");
                    if written.size().is_some() {
                        framed
                            .write_all(buf.filled())
                            .await
                            .expect("write deactivation-reactivation sequence step");
                    }
                    if let connector::connection_activation::ConnectionActivationState::Finalized {
                        share_id,
                        enable_server_pointer,
                        pointer_software_rendering,
                        static_channel_chunk_size,
                        ..
                    } = connection_activation.connection_activation_state()
                    {
                        assert!(stage.reactivate(
                            connection_activation.io_channel_id(),
                            connection_activation.user_channel_id(),
                            share_id,
                            enable_server_pointer,
                            pointer_software_rendering,
                            static_channel_chunk_size,
                        ));
                        break;
                    }
                }
                let reactivated = ctx_rx.recv().await.expect("reactivated connection's display context");
                assert!(Arc::ptr_eq(&first.display_suppressed, &reactivated.display_suppressed));
                assert!(Arc::ptr_eq(&first.autodetect.rtt, &reactivated.autodetect.rtt));
                assert!(Arc::ptr_eq(
                    &first.autodetect.baseline_rtt,
                    &reactivated.autodetect.baseline_rtt
                ));
                assert!(Arc::ptr_eq(
                    &first.autodetect.bandwidth,
                    &reactivated.autodetect.bandwidth
                ));
                assert!(Arc::ptr_eq(
                    &first.autodetect.bandwidth_generation,
                    &reactivated.autodetect.bandwidth_generation
                ));

                // Leave every handle away from its initial value, as a
                // connection that ends minimized after a full measurement would.
                first.display_suppressed.store(true, Ordering::Relaxed);
                first.autodetect.bandwidth.store(50_000, Ordering::Relaxed);
                first.autodetect.bandwidth_generation.store(3, Ordering::Release);
                disconnect(stage, framed).await;

                let (stage, _, framed) = connect(server_addr).await;
                let second = ctx_rx.recv().await.expect("second connection's display context");
                assert!(!Arc::ptr_eq(&first.display_suppressed, &second.display_suppressed));
                assert!(!Arc::ptr_eq(&first.autodetect.rtt, &second.autodetect.rtt));
                assert!(!Arc::ptr_eq(
                    &first.autodetect.baseline_rtt,
                    &second.autodetect.baseline_rtt
                ));
                assert!(!Arc::ptr_eq(&first.autodetect.bandwidth, &second.autodetect.bandwidth));
                assert!(!Arc::ptr_eq(
                    &first.autodetect.bandwidth_generation,
                    &second.autodetect.bandwidth_generation
                ));
                assert!(!second.display_suppressed.load(Ordering::Relaxed));
                assert_eq!(second.autodetect.rtt.load(Ordering::Relaxed), u32::MAX);
                assert_eq!(second.autodetect.baseline_rtt.load(Ordering::Relaxed), u32::MAX);
                assert_eq!(second.autodetect.bandwidth.load(Ordering::Relaxed), u32::MAX);
                assert_eq!(second.autodetect.bandwidth_generation.load(Ordering::Acquire), 0);
                disconnect(stage, framed).await;

                ev.send(ServerEvent::Quit("bye".into())).unwrap();
            });

            tokio::try_join!(server, client).expect("join");
        })
        .await;

    /// Hands every context it receives to the test, and draws whatever the
    /// test sends on `updates`.
    struct RecordingDisplay {
        contexts: UnboundedSender<DisplayContext>,
        updates: DisplayUpdatesRx,
    }

    #[async_trait::async_trait]
    impl RdpServerDisplay for RecordingDisplay {
        async fn size(&mut self) -> DesktopSize {
            DesktopSize {
                width: DESKTOP_WIDTH,
                height: DESKTOP_HEIGHT,
            }
        }

        async fn updates(&mut self, ctx: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
            self.contexts.send(ctx).expect("the test is still receiving");
            Ok(Box::new(TestDisplayUpdates {
                rx: Arc::clone(&self.updates),
            }))
        }
    }

    async fn connect(
        server_addr: SocketAddr,
    ) -> (
        ActiveStage,
        connector::connection_activation::ConnectionActivationFactory,
        Framed<TokioStream<TlsStream<TcpStream>>>,
    ) {
        let (framed, connection_result) = connect_client(server_addr, |client_addr| {
            connector::ClientConnector::new(default_client_config(), client_addr)
        })
        .await;
        let stage = ActiveStageBuilder {
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
        (stage, connection_result.activation_factory, framed)
    }

    async fn disconnect(stage: ActiveStage, mut framed: Framed<TokioStream<TlsStream<TcpStream>>>) {
        for out in stage.graceful_shutdown().expect("shutdown") {
            if let ActiveStageOutput::ResponseFrame(frame) = out {
                framed.write_all(&frame).await.expect("write shutdown frame");
            }
        }
        while framed.read_pdu().await.is_ok() {}
    }
}

type DisplayUpdatesRx = Arc<Mutex<UnboundedReceiver<DisplayUpdate>>>;

struct TestDisplayUpdates {
    rx: DisplayUpdatesRx,
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for TestDisplayUpdates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
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

    async fn updates(&mut self, _: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
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

#[derive(Debug)]
enum RdpdrFixtureOperation {
    UnsupportedCreate,
    #[cfg(windows)]
    CreateFile {
        path: String,
    },
    #[cfg(windows)]
    ReadFile {
        path: String,
        response_lengths: Vec<u32>,
    },
}

#[derive(Debug)]
enum RdpdrFixturePhase {
    AwaitClientAnnounce,
    AwaitClientName,
    AwaitClientCapabilities,
    AwaitDeviceAnnouncement,
    AwaitCreateCompletion,
    #[cfg(windows)]
    AwaitReadCreateCompletion,
    #[cfg(windows)]
    AwaitFirstReadCompletion {
        file_id: u32,
    },
    #[cfg(windows)]
    AwaitSecondReadCompletion,
    Complete,
}

#[derive(Debug)]
struct RdpdrFixtureState {
    phase: RdpdrFixturePhase,
    operation: RdpdrFixtureOperation,
    announced_device_id: Option<u32>,
    completion_status: Option<NtStatus>,
}

impl RdpdrFixtureState {
    fn new(operation: RdpdrFixtureOperation) -> Self {
        Self {
            phase: RdpdrFixturePhase::AwaitClientAnnounce,
            operation,
            announced_device_id: None,
            completion_status: None,
        }
    }

    fn start() -> Vec<SvcMessage> {
        vec![SvcMessage::from(RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR_12,
            client_id: RDPDR_CLIENT_ID,
            kind: VersionAndIdPduKind::ServerAnnounceRequest,
        }))]
    }

    fn process(&mut self, payload: &[u8]) -> pdu::PduResult<Vec<SvcMessage>> {
        match self.phase {
            RdpdrFixturePhase::AwaitClientAnnounce => {
                self.phase = RdpdrFixturePhase::AwaitClientName;
                Ok(vec![server_capabilities()])
            }
            RdpdrFixturePhase::AwaitClientName => {
                self.phase = RdpdrFixturePhase::AwaitClientCapabilities;
                Ok(Vec::new())
            }
            RdpdrFixturePhase::AwaitClientCapabilities => {
                self.phase = RdpdrFixturePhase::AwaitDeviceAnnouncement;
                Ok(vec![
                    SvcMessage::from(RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
                        version_major: VERSION_MAJOR,
                        version_minor: VERSION_MINOR_12,
                        client_id: RDPDR_CLIENT_ID,
                        kind: VersionAndIdPduKind::ServerClientIdConfirm,
                    })),
                    SvcMessage::from(RdpdrPdu::UserLoggedon),
                ])
            }
            RdpdrFixturePhase::AwaitDeviceAnnouncement => {
                let device_id = read_u32(payload, 12);
                self.announced_device_id = Some(device_id);

                let mut messages = vec![SvcMessage::from(RdpdrPdu::ServerDeviceAnnounceResponse(
                    ServerDeviceAnnounceResponse {
                        device_id,
                        result_code: NtStatus::SUCCESS,
                    },
                ))];
                messages.extend(self.start_operation(device_id));
                Ok(messages)
            }
            RdpdrFixturePhase::AwaitCreateCompletion => {
                self.completion_status = Some(read_status(payload));
                self.phase = RdpdrFixturePhase::Complete;
                Ok(Vec::new())
            }
            #[cfg(windows)]
            RdpdrFixturePhase::AwaitReadCreateCompletion => {
                self.completion_status = Some(read_status(payload));
                let file_id = read_u32(payload, 16);
                self.phase = RdpdrFixturePhase::AwaitFirstReadCompletion { file_id };
                Ok(vec![read_request(
                    self.announced_device_id.expect("announced device"),
                    file_id,
                    RDPDR_COMPLETION_ID + 1,
                    0,
                )])
            }
            #[cfg(windows)]
            RdpdrFixturePhase::AwaitFirstReadCompletion { file_id } => {
                self.record_read_response(payload);
                self.phase = RdpdrFixturePhase::AwaitSecondReadCompletion;
                Ok(vec![read_request(
                    self.announced_device_id.expect("announced device"),
                    file_id,
                    RDPDR_COMPLETION_ID + 2,
                    u64::from(RDPDR_READ_LENGTH),
                )])
            }
            #[cfg(windows)]
            RdpdrFixturePhase::AwaitSecondReadCompletion => {
                self.record_read_response(payload);
                self.phase = RdpdrFixturePhase::Complete;
                Ok(Vec::new())
            }
            RdpdrFixturePhase::Complete => Ok(Vec::new()),
        }
    }

    fn start_operation(&mut self, device_id: u32) -> Vec<SvcMessage> {
        match &self.operation {
            RdpdrFixtureOperation::UnsupportedCreate => {
                self.phase = RdpdrFixturePhase::AwaitCreateCompletion;
                vec![create_request(device_id, "unsupported.txt", 1)]
            }
            #[cfg(windows)]
            RdpdrFixtureOperation::CreateFile { path } => {
                self.phase = RdpdrFixturePhase::AwaitCreateCompletion;
                vec![create_request(device_id, path, 2)]
            }
            #[cfg(windows)]
            RdpdrFixtureOperation::ReadFile { path, .. } => {
                self.phase = RdpdrFixturePhase::AwaitReadCreateCompletion;
                vec![create_request(device_id, path, 1)]
            }
        }
    }

    #[cfg(windows)]
    fn record_read_response(&mut self, payload: &[u8]) {
        self.completion_status = Some(read_status(payload));
        let RdpdrFixtureOperation::ReadFile { response_lengths, .. } = &mut self.operation else {
            unreachable!("only the read fixture receives read completions");
        };
        response_lengths.push(read_u32(payload, 16));
    }
}

#[derive(Debug)]
struct RdpdrFixtureFactory {
    state: Arc<StdMutex<RdpdrFixtureState>>,
}

impl RdpdrFixtureFactory {
    fn new(operation: RdpdrFixtureOperation) -> Self {
        Self {
            state: Arc::new(StdMutex::new(RdpdrFixtureState::new(operation))),
        }
    }

    fn state(&self) -> Arc<StdMutex<RdpdrFixtureState>> {
        Arc::clone(&self.state)
    }
}

impl StaticChannelFactory for RdpdrFixtureFactory {
    fn attach(&self, acceptor: &mut Acceptor) {
        acceptor.attach_static_channel(RdpdrFixture {
            state: Arc::clone(&self.state),
        });
    }
}

#[derive(Debug)]
struct RdpdrFixture {
    state: Arc<StdMutex<RdpdrFixtureState>>,
}

impl_as_any!(RdpdrFixture);

impl SvcProcessor for RdpdrFixture {
    fn channel_name(&self) -> gcc::ChannelName {
        Rdpdr::NAME
    }

    fn start(&mut self) -> pdu::PduResult<Vec<SvcMessage>> {
        Ok(RdpdrFixtureState::start())
    }

    fn process(&mut self, payload: &[u8]) -> pdu::PduResult<Vec<SvcMessage>> {
        self.state.lock().expect("fixture state").process(payload)
    }
}

impl SvcServerProcessor for RdpdrFixture {}

#[derive(Debug)]
struct UnsupportedRdpdrBackend;

impl_as_any!(UnsupportedRdpdrBackend);

impl RdpdrBackend for UnsupportedRdpdrBackend {
    fn handle_server_device_announce_response(&mut self, _: ServerDeviceAnnounceResponse) -> pdu::PduResult<()> {
        Ok(())
    }

    fn handle_scard_call(
        &mut self,
        _: ironrdp_rdpdr::pdu::efs::DeviceControlRequest<ScardIoCtlCode>,
        _: ScardCall,
    ) -> pdu::PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> pdu::PduResult<Vec<SvcMessage>> {
        let ServerDriveIoRequest::ServerCreateDriveRequest(request) = req else {
            unreachable!("fixture only issues create requests");
        };
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
            DeviceCreateResponse {
                device_io_reply: DeviceIoResponse::new(request.device_io_request, NtStatus::NOT_SUPPORTED),
                file_id: 0,
                information: ironrdp_rdpdr::pdu::efs::Information::file_superseded(),
            },
        ))])
    }
}

struct TestRdpdrBackendFactory;

impl RdpdrBackendFactory for TestRdpdrBackendFactory {
    fn build_rdpdr_backend(&self) -> ironrdp_rdpdr::RdpdrBackendFactoryResult<RdpdrBackendProduct> {
        Ok(RdpdrBackendProduct::new(
            Box::new(UnsupportedRdpdrBackend),
            vec![RdpdrDrive::new(RDPDR_DEVICE_ID, "test".to_owned())],
        ))
    }
}

fn test_rdpdr_channel() -> Rdpdr {
    rdpdr_channel(&TestRdpdrBackendFactory)
}

fn rdpdr_channel(factory: &dyn RdpdrBackendFactory) -> Rdpdr {
    let (backend, initial_drives) = factory.build_rdpdr_backend().expect("build RDPDR backend").into_parts();
    Rdpdr::new(backend, "IronRDP".to_owned())
        .with_drives(Some(initial_drives.into_iter().map(RdpdrDrive::into_parts).collect()))
}

fn server_capabilities() -> SvcMessage {
    let mut capabilities = Capabilities::new();
    capabilities.add_drive();
    SvcMessage::from(RdpdrPdu::CoreCapability(CoreCapability {
        capabilities: capabilities.clone_inner(),
        kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
    }))
}

fn create_request(device_id: u32, path: &str, create_disposition: u32) -> SvcMessage {
    let mut request = encode_vec(&RdpdrPdu::DeviceIoRequest(DeviceIoRequest {
        device_id,
        file_id: 0,
        completion_id: RDPDR_COMPLETION_ID,
        major_function: MajorFunction::Create,
        minor_function: MinorFunction::from(0),
    }))
    .expect("encode create request");
    let path = path
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    request.extend_from_slice(&0xC000_0000u32.to_le_bytes()); // DesiredAccess
    request.extend_from_slice(&0u64.to_le_bytes()); // AllocationSize
    request.extend_from_slice(&0x80u32.to_le_bytes()); // FileAttributes
    request.extend_from_slice(&1u32.to_le_bytes()); // SharedAccess
    request.extend_from_slice(&create_disposition.to_le_bytes()); // CreateDisposition
    request.extend_from_slice(&0x40u32.to_le_bytes()); // CreateOptions
    request.extend_from_slice(&u32::try_from(path.len()).expect("path length fits u32").to_le_bytes());
    request.extend_from_slice(&path);
    SvcMessage::from(request)
}

#[cfg(windows)]
fn read_request(device_id: u32, file_id: u32, completion_id: u32, offset: u64) -> SvcMessage {
    let mut request = encode_vec(&RdpdrPdu::DeviceIoRequest(DeviceIoRequest {
        device_id,
        file_id,
        completion_id,
        major_function: MajorFunction::Read,
        minor_function: MinorFunction::from(0),
    }))
    .expect("encode read request");
    request.extend_from_slice(&RDPDR_READ_LENGTH.to_le_bytes());
    request.extend_from_slice(&offset.to_le_bytes());
    request.extend_from_slice(&[0; 20]);
    SvcMessage::from(request)
}

fn read_u32(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        payload[offset..offset + 4]
            .try_into()
            .expect("RDPDR fixture received a complete response"),
    )
}

fn read_status(payload: &[u8]) -> NtStatus {
    NtStatus::from(read_u32(payload, 12))
}

async fn drive_rdpdr_until_complete(
    mut stage: ActiveStage,
    mut framed: Framed<TokioStream<TlsStream<TcpStream>>>,
    display_tx: UnboundedSender<DisplayUpdate>,
    fixture_state: Arc<StdMutex<RdpdrFixtureState>>,
) -> (ActiveStage, Framed<TokioStream<TlsStream<TcpStream>>>) {
    let _display_tx = display_tx;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, DESKTOP_WIDTH, DESKTOP_HEIGHT);
    let deadline = Instant::now() + Duration::from_secs(10);

    while !matches!(
        fixture_state.lock().expect("fixture state").phase,
        RdpdrFixturePhase::Complete
    ) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "RDPDR fixture did not complete");
        let read_result = tokio::time::timeout(remaining.min(Duration::from_millis(250)), framed.read_pdu()).await;
        let Ok(frame_result) = read_result else {
            continue;
        };
        let (action, frame) = frame_result.expect("read RDPDR frame");
        let outputs = stage.process(&mut image, action, &frame).expect("process RDPDR frame");
        for output in outputs {
            if let ActiveStageOutput::ResponseFrame(frame) = output {
                framed.write_all(&frame).await.expect("write RDPDR response");
            }
        }
        tokio::task::yield_now().await;
    }

    let rdpdr_state = fixture_state.lock().expect("fixture state");
    assert_eq!(rdpdr_state.announced_device_id, Some(RDPDR_DEVICE_ID));
    match &rdpdr_state.operation {
        RdpdrFixtureOperation::UnsupportedCreate => {
            assert_eq!(rdpdr_state.completion_status, Some(NtStatus::NOT_SUPPORTED));
        }
        #[cfg(windows)]
        RdpdrFixtureOperation::CreateFile { .. } | RdpdrFixtureOperation::ReadFile { .. } => {
            assert_eq!(rdpdr_state.completion_status, Some(NtStatus::SUCCESS));
        }
    }
    drop(rdpdr_state);
    (stage, framed)
}

#[cfg(windows)]
fn test_directory(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ironrdp-rdpdr-{name}-{}", uuid::Uuid::new_v4()))
}

#[cfg(windows)]
fn volume_root(path: &Path) -> &Path {
    path.ancestors().last().expect("test directory has a volume root")
}

#[cfg(windows)]
fn volume_relative_path(directory: &Path, file_name: &str) -> String {
    let relative = directory
        .strip_prefix(volume_root(directory))
        .expect("test directory is beneath its volume root");
    format!(r"\{}\{file_name}", relative.display()).replace('/', r"\")
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
        Vec::new(),
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
    static_channel_factories: Vec<Box<dyn StaticChannelFactory>>,
    server_udp_addr: Option<SocketAddr>,
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
    client_server_impl(
        None,
        client_config,
        static_channel_factories,
        server_udp_addr,
        connector_factory,
        clientfn,
    )
    .await;
}

/// With `stale_events` set, the server is driven the way an embedder drives
/// it: the events are queued first, then one connection is served through
/// `run_connection`, with no `run()` idle loop in between to discard them.
async fn client_server_impl<F, Fut, C>(
    stale_events: Option<Vec<ServerEvent>>,
    client_config: connector::Config,
    static_channel_factories: Vec<Box<dyn StaticChannelFactory>>,
    server_udp_addr: Option<SocketAddr>,
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
    let mut server_builder = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_tls(acceptor)
        .with_input_handler(TestInputHandler)
        .with_display_handler(TestDisplay {
            rx: Arc::new(Mutex::new(display_rx)),
        });
    for factory in static_channel_factories {
        server_builder = server_builder.with_static_channel_factory(factory);
    }
    if let Some(udp_addr) = server_udp_addr {
        server_builder = server_builder.with_udp_transport(udp_addr);
    }
    let mut server = server_builder.build();
    server.set_credentials(Some(server::Credentials {
        username: USERNAME.into(),
        password: PASSWORD.into(),
        domain: None,
    }));
    let ev = server.event_sender().clone();
    let echo_handle = server.echo_handle().clone();

    let direct_listener = match stale_events {
        Some(events) => {
            for event in events {
                ev.send(event).unwrap();
            }
            Some(TcpListener::bind(("127.0.0.1", 0)).await.expect("bind listener"))
        }
        None => None,
    };
    let direct_addr = direct_listener
        .as_ref()
        .map(|listener| listener.local_addr().expect("listener address"));

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let server = tokio::task::spawn_local(async move {
                match direct_listener {
                    Some(listener) => {
                        let (stream, _) = listener.accept().await.expect("accept connection");
                        server.run_connection(stream).await.unwrap();
                    }
                    None => server.run().await.unwrap(),
                }
            });

            let client = tokio::task::spawn_local(async move {
                let server_addr = match direct_addr {
                    Some(addr) => addr,
                    None => local_addr_of(&ev).await,
                };
                let (upgraded_framed, connection_result) = connect_client(server_addr, |client_addr| {
                    connector_factory(connector::ClientConnector::new(client_config, client_addr))
                })
                .await;

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
                // Nothing is left to receive it when `run_connection` already returned.
                let _ = ev.send(ServerEvent::Quit("bye".into()));
            });

            tokio::try_join!(server, client).expect("join");
        })
        .await;
}

/// EGFX moves onto the UDP tunnel with Soft-Sync (MS-RDPEDYC 3.1.5.3), end to end.
///
/// The server sends the Soft-Sync Request when EGFX first has data to send, and
/// from then on every server message on that channel goes over the tunnel
/// (3.3.5.3.1): the batch that triggered the request, and the channel's reply to
/// a client message. The IronRDP client rejects TCP data for a channel it moved
/// to the tunnel, so any EGFX data sent over TCP after the request fails the test.
///
/// The client writes on the tunnel as soon as it has sent the Soft-Sync
/// Response, so its first tunnel data can reach the server before the response
/// does. The test forces that order and checks the server still processes it.
///
/// Soft-Sync has no tunnel type that moves a channel back to TCP (2.2.5.1),
/// and the tunnel lasts as long as the connection (MS-RDPEMT 1.3.3), so when
/// the tunnel closes under EGFX the server has to end the connection.
#[tokio::test]
async fn egfx_moves_onto_the_udp_tunnel_with_soft_sync() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    // The server binds its sideband socket to the configured port, so the
    // client has to know that port before the server reports it.
    let udp_port = std::net::UdpSocket::bind("127.0.0.1:0")
        .and_then(|socket| socket.local_addr())
        .expect("pick a free UDP port")
        .port();
    let udp_addr: SocketAddr = ([127, 0, 0, 1], udp_port).into();

    let cert_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-cert.pem");
    let key_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-key.pem");
    let identity = TlsIdentityCtx::init_from_paths(&cert_path, &key_path).expect("failed to init TLS identity");
    let acceptor = identity.make_acceptor().expect("failed to build TLS acceptor");

    let (caps_tx, mut caps_rx) = mpsc::unbounded_channel();
    let (_display_tx, display_rx) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_tls(acceptor)
        .with_input_handler(TestInputHandler)
        .with_display_handler(TestDisplay {
            rx: Arc::new(Mutex::new(display_rx)),
        })
        .with_gfx_factory(Some(Box::new(TestGfxFactory { caps_tx })))
        .with_udp_transport(udp_addr)
        .build();
    server.set_credentials(Some(server::Credentials {
        username: USERNAME.into(),
        password: PASSWORD.into(),
        domain: None,
    }));
    let ev = server.event_sender().clone();

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

                let egfx = TestEgfxClient::default();
                let egfx_channel = Arc::clone(&egfx.channel_id);
                let egfx_received = Arc::clone(&egfx.received);
                let client_config = connector::Config {
                    support_dyn_vc_gfx_protocol: true,
                    multitransport_flags: Some(
                        gcc::MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR
                            | gcc::MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP,
                    ),
                    ..default_client_config()
                };
                let mut connector = connector::ClientConnector::new(client_config, client_addr)
                    .with_static_channel(DrdynvcClient::new().with_dynamic_channel(egfx));

                let mut framed = ironrdp_tokio::TokioFramed::new(tcp_stream);
                let should_upgrade = ironrdp_async::connect_begin(&mut framed, &mut connector)
                    .await
                    .expect("begin connection");
                let (upgraded_stream, tls_cert) = ironrdp_tls::upgrade_with_certificate_validation(
                    framed.into_inner_no_leftover(),
                    "localhost",
                    ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate,
                )
                .await
                .expect("TLS upgrade");
                let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
                let mut framed = ironrdp_tokio::TokioFramed::new(upgraded_stream);
                let server_public_key =
                    ironrdp_tls::extract_tls_server_public_key(&tls_cert).expect("extract server public key");

                let mut tunnel = None;
                let connection_result = ironrdp_async::connect_finalize_with_multitransport(
                    upgraded,
                    connector,
                    &mut framed,
                    &mut ironrdp_tokio::reqwest::ReqwestNetworkClient::new(),
                    "localhost".into(),
                    server_public_key.to_owned(),
                    None,
                    async |request, soft_sync| {
                        assert!(soft_sync, "both peers advertised SOFT_SYNC_TCP_TO_UDP");
                        let mut bootstrap = ironrdp_rdpeudp_tokio::MultitransportBootstrap::new(request);
                        bootstrap
                            .connect(
                                udp_addr,
                                "localhost".into(),
                                ironrdp_rdpeudp::ConnectionConfig::default(),
                                ironrdp_rdpeudp_tokio::UdpTlsConfig::new("localhost".into()),
                            )
                            .await
                            .expect("UDP tunnel handshake");
                        tunnel = bootstrap.take_transport();
                        Ok(connector::MultitransportResult::Success)
                    },
                )
                .await
                .expect("finalize connection");
                let mut tunnel = tunnel.expect("the server sent an Initiate Multitransport Request");

                let mut stage = ActiveStageBuilder {
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
                stage.enable_reliable_udp_dvc_tunnel().expect("DRDYNVC is present");
                let mut image = DecodedImage::new(PixelFormat::RgbA32, DESKTOP_WIDTH, DESKTOP_HEIGHT);

                // The client advertises its capabilities over TCP when the channel
                // opens, as a real client does; the server handler seeing them means
                // the server has the channel open too.
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        tokio::select! {
                            caps = caps_rx.recv() => {
                                caps.expect("capabilities reached the EGFX handler");
                                break;
                            }
                            pdu = framed.read_pdu() => {
                                let (action, frame) = pdu.expect("read PDU");
                                for output in stage.process(&mut image, action, &frame).expect("stage process") {
                                    if let ActiveStageOutput::ResponseFrame(frame) = output {
                                        framed.write_all(&frame).await.expect("write response frame");
                                    }
                                }
                            }
                        }
                    }
                })
                .await
                .expect("the server opened the EGFX channel");
                let egfx_channel_id = egfx_channel
                    .lock()
                    .unwrap()
                    .expect("the client opened the EGFX channel");

                let batch = b"first egfx batch".to_vec();
                let messages = ironrdp_dvc::encode_dvc_messages(
                    egfx_channel_id,
                    vec![Box::new(RawDvcPayload(batch.clone()))],
                    ironrdp::svc::ChannelFlags::empty(),
                )
                .expect("encode EGFX batch");
                ev.send(ServerEvent::Egfx(ironrdp_server::EgfxServerMessage::SendMessages {
                    messages,
                }))
                .unwrap();

                // Process TCP until the Soft-Sync Request has moved the channel,
                // holding back the Soft-Sync Response it produced.
                let held_response = tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let (action, frame) = framed.read_pdu().await.expect("read PDU");
                        let mut responses = Vec::new();
                        for output in stage.process(&mut image, action, &frame).expect("stage process") {
                            if let ActiveStageOutput::ResponseFrame(frame) = output {
                                responses.push(frame);
                            }
                        }
                        if stage.dvc_tunnel_for_channel(egfx_channel_id)
                            == Some(ironrdp_dvc::pdu::SoftSyncTunnelType::RELIABLE_UDP)
                        {
                            break responses;
                        }
                        for frame in responses {
                            framed.write_all(&frame).await.expect("write response frame");
                        }
                    }
                })
                .await
                .expect("the server sent a Soft-Sync Request for EGFX");

                // Client data on the tunnel that reaches the server ahead of the
                // Soft-Sync Response, and that the EGFX channel answers: a second
                // capabilities advertisement, which a client sends to reset its
                // decoder.
                let caps = ironrdp_dvc::pdu::DrdynvcClientPdu::Data(ironrdp_dvc::pdu::DrdynvcDataPdu::Data(
                    ironrdp_dvc::pdu::DataPdu::new(egfx_channel_id, encode_vec(&v8_capabilities()).unwrap()),
                ));
                tunnel
                    .send(encode_vec(&caps).unwrap())
                    .await
                    .expect("send on the tunnel");
                tokio::time::sleep(Duration::from_millis(200)).await;
                for frame in held_response {
                    framed.write_all(&frame).await.expect("write Soft-Sync Response");
                }

                // Over TCP, the confirmation of the first capabilities. Over the
                // tunnel, the triggering batch and the second confirmation.
                tokio::time::timeout(Duration::from_secs(10), async {
                    while egfx_received.lock().unwrap().len() < 3 {
                        tokio::select! {
                            payload = tunnel.recv() => {
                                let payload = payload.expect("tunnel open");
                                stage
                                    .process_dvc_tunnel(&mut image, ironrdp_dvc::pdu::SoftSyncTunnelType::RELIABLE_UDP, &payload)
                                    .expect("DVC data for the tunneled channel");
                            }
                            pdu = framed.read_pdu() => {
                                let (action, frame) = pdu.expect("read PDU");
                                for output in stage.process(&mut image, action, &frame).expect("stage process") {
                                    if let ActiveStageOutput::ResponseFrame(frame) = output {
                                        framed.write_all(&frame).await.expect("write response frame");
                                    }
                                }
                            }
                        }
                    }
                })
                .await
                .expect("EGFX batch and capabilities reply arrived on the tunnel");
                assert_eq!(
                    egfx_received.lock().unwrap()[1],
                    batch,
                    "the batch that triggered the request goes over the tunnel"
                );

                tokio::time::timeout(Duration::from_secs(10), caps_rx.recv())
                    .await
                    .expect("the server processed the early tunnel data")
                    .expect("capabilities reached the EGFX handler");

                // With EGFX on the tunnel there is no way back to TCP, so the
                // server ends the connection when the tunnel closes. Graphics
                // still being sent is what shows the server the tunnel is gone.
                drop(tunnel);
                let messages = ironrdp_dvc::encode_dvc_messages(
                    egfx_channel_id,
                    vec![Box::new(RawDvcPayload(b"after the tunnel closed".to_vec()))],
                    ironrdp::svc::ChannelFlags::empty(),
                )
                .expect("encode EGFX batch");
                ev.send(ServerEvent::Egfx(ironrdp_server::EgfxServerMessage::SendMessages {
                    messages,
                }))
                .unwrap();
                tokio::time::timeout(Duration::from_secs(20), async {
                    while let Ok(pdu) = framed.read_pdu().await {
                        debug!(?pdu);
                    }
                })
                .await
                .expect("the server ended the connection once the tunnel closed");
                ev.send(ServerEvent::Quit("bye".into())).unwrap();
            });

            tokio::try_join!(server, client).expect("join");
        })
        .await;
}

/// Server EGFX handler that reports the client's advertised capabilities.
struct TestGfxFactory {
    caps_tx: UnboundedSender<()>,
}

impl ServerEventSender for TestGfxFactory {
    fn set_sender(&mut self, _sender: UnboundedSender<ServerEvent>) {}
}

impl GfxServerFactory for TestGfxFactory {
    fn build_gfx_handler(&self) -> Box<dyn ironrdp_egfx::server::GraphicsPipelineHandler> {
        Box::new(TestGfxHandler {
            caps_tx: self.caps_tx.clone(),
        })
    }
}

struct TestGfxHandler {
    caps_tx: UnboundedSender<()>,
}

impl ironrdp_egfx::server::GraphicsPipelineHandler for TestGfxHandler {
    fn capabilities_advertise(&mut self, _pdu: &CapabilitiesAdvertisePdu) {
        let _ = self.caps_tx.send(());
    }

    fn on_ready(&mut self, _negotiated: &CapabilitySet) {}
}

/// Client end of the EGFX channel that records what the server sends on it.
#[derive(Default)]
struct TestEgfxClient {
    channel_id: Arc<StdMutex<Option<u32>>>,
    received: Arc<StdMutex<Vec<Vec<u8>>>>,
}

impl_as_any!(TestEgfxClient);

impl DvcProcessor for TestEgfxClient {
    fn channel_name(&self) -> &str {
        "Microsoft::Windows::RDS::Graphics"
    }

    fn start(&mut self, channel_id: u32) -> pdu::PduResult<Vec<DvcMessage>> {
        *self.channel_id.lock().unwrap() = Some(channel_id);
        Ok(vec![Box::new(v8_capabilities())])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> pdu::PduResult<Vec<DvcMessage>> {
        self.received.lock().unwrap().push(payload.to_vec());
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for TestEgfxClient {}

fn v8_capabilities() -> GfxPdu {
    GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::empty(),
    }]))
}

struct RawDvcPayload(Vec<u8>);

impl ironrdp::core::Encode for RawDvcPayload {
    fn encode(&self, dst: &mut ironrdp::core::WriteCursor<'_>) -> ironrdp::core::EncodeResult<()> {
        ironrdp::core::ensure_size!(in: dst, size: self.0.len());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RawDvcPayload"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for RawDvcPayload {}

/// By default a connection is sent an auto-reconnect cookie as soon as it is
/// active.
#[tokio::test]
async fn an_auto_reconnect_cookie_is_sent_at_activation_by_default() {
    let (at_activation, _) = auto_reconnect_session(false).await;
    assert!(at_activation, "the cookie goes to every connection at activation");
}

/// On request, a connection gets no cookie at activation, only once the
/// embedder issues one, for instance after its own logon screen.
#[tokio::test]
async fn an_auto_reconnect_cookie_on_request_waits_for_the_embedder() {
    let (at_activation, after_request) = auto_reconnect_session(true).await;
    assert!(!at_activation, "no cookie before the embedder asks for one");
    assert!(after_request, "the cookie the embedder issued reaches the client");
}

/// The hourly cookie update keeps reaching a connection that was issued a cookie, also
/// after a Deactivation-Reactivation pass on that connection. Time is paused once the
/// connection is up, so the hour passes at once without tripping the server's own handshake
/// timeout.
#[tokio::test]
async fn the_hourly_auto_reconnect_update_survives_a_reactivation() {
    let update_after_reactivation = with_auto_reconnect_server(true, async |server_addr, cookie_handle, display_tx| {
        let mut probe = CookieProbe::connect(server_addr, None).await;
        cookie_handle
            .set(Some(ServerAutoReconnect {
                logon_id: 1,
                random_bits: [0x33; 16],
            }))
            .expect("issue the cookie");
        assert!(
            probe.cookie_within(Duration::from_secs(5)).await,
            "the issued cookie arrives"
        );

        probe.reactivate(&display_tx).await;

        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(60 * 60 + 1)).await;
        let update = probe.cookie_within(Duration::from_secs(30)).await;
        probe.disconnect().await;
        update
    })
    .await;

    assert!(
        update_after_reactivation,
        "the hourly update still goes to the connection that holds a cookie"
    );
}

/// The hourly cookie update goes only to a connection that was issued a cookie: a
/// connection the embedder never vouched for gets none, even when an earlier connection
/// was issued one, and also after its own Deactivation-Reactivation pass.
#[tokio::test]
async fn the_hourly_auto_reconnect_update_skips_a_connection_that_was_never_issued_a_cookie() {
    let update = with_auto_reconnect_server(true, async |server_addr, cookie_handle, display_tx| {
        let mut issued = CookieProbe::connect(server_addr, None).await;
        cookie_handle
            .set(Some(ServerAutoReconnect {
                logon_id: 1,
                random_bits: [0x44; 16],
            }))
            .expect("issue the cookie");
        assert!(
            issued.cookie_within(Duration::from_secs(5)).await,
            "the issued cookie arrives"
        );
        issued.disconnect().await;

        let mut unvouched = CookieProbe::connect(server_addr, None).await;
        unvouched.reactivate(&display_tx).await;

        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(60 * 60 + 1)).await;
        let update = unvouched.cookie_within(Duration::from_secs(30)).await;
        unvouched.disconnect().await;
        update
    })
    .await;

    assert!(!update, "no cookie goes to a connection the embedder never vouched for");
}

/// Connects a client to `server_addr` and completes the handshake, returning the state
/// needed to drive the connection.
///
/// `cookie`, if given, is presented as an auto-reconnect cookie in the handshake.
async fn connect_active_client(
    server_addr: SocketAddr,
    client_config: connector::Config,
    cookie: Option<ServerAutoReconnect>,
) -> (
    ActiveStage,
    connector::connection_activation::ConnectionActivationFactory,
    Framed<TokioStream<TlsStream<TcpStream>>>,
) {
    let (upgraded_framed, connection_result) = connect_client(server_addr, |client_addr| {
        let connector = connector::ClientConnector::new(client_config, client_addr);
        match cookie {
            Some(cookie) => connector.with_auto_reconnect_cookie(cookie),
            None => connector,
        }
    })
    .await;

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

    (active_stage, activation_factory, upgraded_framed)
}

/// A server that holds an auto-reconnect cookie and whose cookies are issued on request or at
/// activation, with the channel that feeds its display.
fn auto_reconnect_server(on_request: bool) -> (RdpServer, UnboundedSender<DisplayUpdate>) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let cert_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-cert.pem");
    let key_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-key.pem");
    let identity = TlsIdentityCtx::init_from_paths(&cert_path, &key_path).expect("failed to init TLS identity");
    let acceptor = identity.make_acceptor().expect("failed to build TLS acceptor");

    let (display_tx, display_rx) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_tls(acceptor)
        .with_input_handler(TestInputHandler)
        .with_display_handler(TestDisplay {
            rx: Arc::new(Mutex::new(display_rx)),
        })
        .with_auto_reconnect_cookie(Some(ServerAutoReconnect {
            logon_id: 1,
            random_bits: [0x5a; 16],
        }))
        .with_auto_reconnect_on_request(on_request)
        .build();
    server.set_credentials(Some(server::Credentials {
        username: USERNAME.into(),
        password: PASSWORD.into(),
        domain: None,
    }));
    (server, display_tx)
}

/// Runs `client` against a server that holds an auto-reconnect cookie and whose cookies are
/// issued on request or at activation. `client` gets the server address, the cookie handle
/// and the channel that feeds the server's display, and the server quits when it returns.
async fn with_auto_reconnect_server<F, Fut, T>(on_request: bool, client: F) -> T
where
    F: FnOnce(SocketAddr, server::AutoReconnectCookieHandle, UnboundedSender<DisplayUpdate>) -> Fut + 'static,
    Fut: Future<Output = T>,
    T: 'static,
{
    let (mut server, display_tx) = auto_reconnect_server(on_request);
    let ev = server.event_sender().clone();
    let cookie_handle = server.auto_reconnect_cookie_handle();

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
                let result = client(server_addr, cookie_handle, display_tx).await;
                ev.send(ServerEvent::Quit("bye".into())).unwrap();
                result
            });

            let (server, client) = tokio::join!(server, client);
            server.expect("server task");
            client.expect("client task")
        })
        .await
}

/// A connected client that reports whether the server sends it an auto-reconnect cookie.
struct CookieProbe {
    stage: ActiveStage,
    activation_factory: connector::connection_activation::ConnectionActivationFactory,
    framed: Framed<TokioStream<TlsStream<TcpStream>>>,
    image: DecodedImage,
}

impl CookieProbe {
    /// Connects to `server_addr`, presenting `cookie` as an auto-reconnect cookie if given.
    async fn connect(server_addr: SocketAddr, cookie: Option<ServerAutoReconnect>) -> Self {
        let (stage, activation_factory, framed) =
            connect_active_client(server_addr, default_client_config(), cookie).await;

        Self {
            stage,
            activation_factory,
            framed,
            image: DecodedImage::new(PixelFormat::RgbA32, DESKTOP_WIDTH, DESKTOP_HEIGHT),
        }
    }

    /// Reads for `window` and reports whether a cookie arrived.
    ///
    /// A cookie that never comes, `Ok(false)`, and a connection that ended, `Err` with the step
    /// that noticed, are told apart, so a test that expects no cookie cannot pass because the
    /// connection died.
    async fn watch_for_cookie(&mut self, window: Duration) -> Result<bool, String> {
        let deadline = Instant::now() + window;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let (action, frame) = match tokio::time::timeout(left, self.framed.read_pdu()).await {
                Err(_elapsed) => break,
                Ok(Ok(pdu)) => pdu,
                Ok(Err(error)) => return Err(format!("read PDU: {error}")),
            };
            let outputs = match self.stage.process(&mut self.image, action, &frame) {
                Ok(outputs) => outputs,
                Err(error) => return Err(format!("stage process: {error}")),
            };
            for output in outputs {
                match output {
                    ActiveStageOutput::AutoReconnectCookie(_) => return Ok(true),
                    ActiveStageOutput::ResponseFrame(frame) => {
                        if let Err(error) = self.framed.write_all(&frame).await {
                            return Err(format!("write response frame: {error}"));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(false)
    }

    /// Whether a cookie arrives while reading for `window`. A connection that ends first fails
    /// the test.
    async fn cookie_within(&mut self, window: Duration) -> bool {
        self.watch_for_cookie(window)
            .await
            .expect("the connection ended while watching for a cookie")
    }

    /// Has the server run a Deactivation-Reactivation Sequence, by resizing its display.
    async fn reactivate(&mut self, display_tx: &UnboundedSender<DisplayUpdate>) {
        display_tx
            .send(DisplayUpdate::Resize(DesktopSize {
                width: 2048,
                height: 2048,
            }))
            .expect("resize the server display");
        loop {
            let (action, frame) = self.framed.read_pdu().await.expect("read PDU");
            let outputs = self
                .stage
                .process(&mut self.image, action, &frame)
                .expect("stage process");
            if outputs
                .iter()
                .any(|output| matches!(output, ActiveStageOutput::DeactivateAll))
            {
                run_reactivation(&mut self.stage, &self.activation_factory, &mut self.framed).await;
                return;
            }
        }
    }

    /// Whether the server accepts the cookie this client presented. An accepted cookie is
    /// answered with a fresh one at activation; a rejected cookie ends the connection.
    async fn cookie_accepted(&mut self, window: Duration) -> bool {
        self.watch_for_cookie(window).await.unwrap_or(false)
    }

    async fn disconnect(mut self) {
        for output in self.stage.graceful_shutdown().expect("shutdown") {
            if let ActiveStageOutput::ResponseFrame(frame) = output {
                self.framed.write_all(&frame).await.expect("write frame");
            }
        }
        while let Ok(pdu) = self.framed.read_pdu().await {
            debug!(?pdu);
        }
    }
}

/// Connects a client to a server holding an auto-reconnect cookie, and
/// reports whether the client received a cookie at activation and whether it
/// received one after the server was handed a new one.
async fn auto_reconnect_session(on_request: bool) -> (bool, bool) {
    with_auto_reconnect_server(on_request, async |server_addr, cookie_handle, _display_tx| {
        let mut probe = CookieProbe::connect(server_addr, None).await;
        let at_activation = probe.cookie_within(Duration::from_secs(1)).await;
        cookie_handle
            .set(Some(ServerAutoReconnect {
                logon_id: 1,
                random_bits: [0xa5; 16],
            }))
            .expect("send the new cookie to the server");
        let after_request = probe.cookie_within(Duration::from_secs(5)).await;
        probe.disconnect().await;
        (at_activation, after_request)
    })
    .await
}

/// A cookie the embedder issues replaces the one before it at once, so a client
/// holding the earlier cookie can no longer reconnect with it (MS-RDPBCGR 5.5), while
/// the newer one still works.
#[tokio::test]
async fn an_auto_reconnect_cookie_the_embedder_issues_invalidates_the_previous_one() {
    let first = ServerAutoReconnect {
        logon_id: 1,
        random_bits: [0x11; 16],
    };
    let second = ServerAutoReconnect {
        logon_id: 1,
        random_bits: [0x22; 16],
    };

    let (first_still_works, second_works) = with_auto_reconnect_server(true, {
        let (first, second) = (first.clone(), second.clone());
        async move |server_addr, cookie_handle, _display_tx| {
            // The embedder vouches for one connection, then for another.
            for cookie in [&first, &second] {
                let mut probe = CookieProbe::connect(server_addr, None).await;
                assert!(
                    !probe.cookie_within(Duration::from_secs(1)).await,
                    "no cookie at activation"
                );
                cookie_handle.set(Some(cookie.clone())).expect("issue the cookie");
                assert!(
                    probe.cookie_within(Duration::from_secs(5)).await,
                    "the issued cookie arrives"
                );
                probe.disconnect().await;
            }

            // A verified cookie is sent another one at activation; a rejected one ends the connection.
            let mut with_first = CookieProbe::connect(server_addr, Some(first)).await;
            let first_still_works = with_first.cookie_accepted(Duration::from_secs(1)).await;
            drop(with_first);

            let mut with_second = CookieProbe::connect(server_addr, Some(second)).await;
            let second_works = with_second.cookie_accepted(Duration::from_secs(1)).await;
            with_second.disconnect().await;

            (first_still_works, second_works)
        }
    })
    .await;

    assert!(!first_still_works, "the earlier client's cookie is no longer accepted");
    assert!(second_works, "the newest cookie is accepted");
}

/// On request, a vouch belongs to the connection that was current when the embedder sent
/// it. One the server never read before that connection was replaced must not give the
/// replacement a cookie, because the replacement has proved nothing. The server is driven the
/// way an embedder drives it, with the vouch queued before the next connection is served.
#[tokio::test]
async fn a_vouch_for_a_replaced_connection_does_not_reach_its_replacement() {
    let (mut server, _display_tx) = auto_reconnect_server(true);
    let cookie_handle = server.auto_reconnect_cookie_handle();
    cookie_handle
        .set(Some(ServerAutoReconnect {
            logon_id: 1,
            random_bits: [0x66; 16],
        }))
        .expect("queue the stale vouch");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind listener");
    let server_addr = listener.local_addr().expect("listener address");

    let local = tokio::task::LocalSet::new();
    let (stale_vouch_reached_it, fresh_vouch_reached_it) = local
        .run_until(async move {
            let server = tokio::task::spawn_local(async move {
                let (stream, _) = listener.accept().await.expect("accept connection");
                server.run_connection(stream).await.unwrap();
            });

            let client = tokio::task::spawn_local(async move {
                let mut probe = CookieProbe::connect(server_addr, None).await;
                let stale = probe.cookie_within(Duration::from_secs(1)).await;
                cookie_handle
                    .set(Some(ServerAutoReconnect {
                        logon_id: 1,
                        random_bits: [0x67; 16],
                    }))
                    .expect("vouch for the connection");
                let fresh = probe.cookie_within(Duration::from_secs(5)).await;
                probe.disconnect().await;
                (stale, fresh)
            });

            let (server, client) = tokio::join!(server, client);
            server.expect("server task");
            client.expect("client task")
        })
        .await;

    assert!(
        !stale_vouch_reached_it,
        "a vouch queued for a connection that is gone must not reach its replacement"
    );
    assert!(
        fresh_vouch_reached_it,
        "a vouch sent for the live connection still reaches it"
    );
}

pub(super) fn default_client_config() -> connector::Config {
    connector::Config {
        desktop_size: DesktopSize {
            width: DESKTOP_WIDTH,
            height: DESKTOP_HEIGHT,
        },
        monitor_layout: None,
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
        keyboard_type: gcc::KeyboardType::IBM_ENHANCED,
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
        enable_audio_capture: false,
        license_cache: None,
        compression_type: None,
        enable_server_pointer: true,
        pointer_software_rendering: true,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
        performance_flags: Default::default(),
        timezone_info: Default::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: pdu::rdp::capability_sets::RailSupportLevel::SUPPORTED,
    }
}

/// Drive a real client through the full handshake against `server_addr`
/// (TLS upgrade, CredSSP, finalization) and return its open transport and the
/// connection result. `make_connector` receives the client's local address.
async fn connect_client(
    server_addr: SocketAddr,
    make_connector: impl FnOnce(SocketAddr) -> connector::ClientConnector,
) -> (Framed<TokioStream<TlsStream<TcpStream>>>, connector::ConnectionResult) {
    let tcp_stream = TcpStream::connect(server_addr).await.expect("TCP connect");
    let client_addr = tcp_stream.local_addr().expect("local_addr");
    let mut framed = ironrdp_tokio::TokioFramed::new(tcp_stream);
    let mut connector = make_connector(client_addr);
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
    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&tls_cert).expect("extract server public key");
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
    (upgraded_framed, connection_result)
}

/// Counts how often each `ConnectionHandler` hook reached the handler.
/// `on_accept` fires before a connection is served, `on_connection_info`
/// mid-connection, `on_disconnected` after it ends.
#[derive(Default)]
struct HookCounts {
    accept: AtomicUsize,
    info: AtomicUsize,
    disconnected: AtomicUsize,
}

impl HookCounts {
    fn get(&self) -> (usize, usize, usize) {
        (
            self.accept.load(Ordering::SeqCst),
            self.info.load(Ordering::SeqCst),
            self.disconnected.load(Ordering::SeqCst),
        )
    }

    /// Poll until `done` holds for the counts, or a generous deadline passes.
    async fn wait_until(&self, done: impl Fn((usize, usize, usize)) -> bool) -> (usize, usize, usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(self.get()) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.get()
    }
}

struct HookRecorder(Arc<HookCounts>);

impl server::ConnectionHandler for HookRecorder {
    fn on_accept(&mut self, _peer: SocketAddr) -> bool {
        self.0.accept.fetch_add(1, Ordering::SeqCst);
        true
    }

    fn on_connection_info(&mut self, _info: &server::ConnectionInfo) {
        self.0.info.fetch_add(1, Ordering::SeqCst);
    }

    fn on_disconnected(
        &mut self,
        _peer: SocketAddr,
        _duration: Duration,
        _error: Option<&server::ServerError>,
    ) -> server::PostConnectionAction {
        self.0.disconnected.fetch_add(1, Ordering::SeqCst);
        server::PostConnectionAction::Continue
    }
}

/// A TLS server under `policy` whose connection handler records into the
/// returned counts. Keep the returned display sender alive for the test: once
/// it drops, the display stream ends and the server closes the session.
fn hook_recording_server(
    policy: server::ConnectionPolicy,
) -> (RdpServer, Arc<HookCounts>, UnboundedSender<DisplayUpdate>) {
    let cert_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-cert.pem");
    let key_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-key.pem");
    let identity = TlsIdentityCtx::init_from_paths(&cert_path, &key_path).expect("failed to init TLS identity");
    let acceptor = identity.make_acceptor().expect("failed to build TLS acceptor");

    let counts = Arc::new(HookCounts::default());
    let (display_tx, display_rx) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_tls(acceptor)
        .with_input_handler(TestInputHandler)
        .with_display_handler(TestDisplay {
            rx: Arc::new(Mutex::new(display_rx)),
        })
        .with_connection_handler(Some(Box::new(HookRecorder(Arc::clone(&counts)))))
        .with_connection_policy(policy)
        .build();
    server.set_credentials(Some(server::Credentials {
        username: USERNAME.into(),
        password: PASSWORD.into(),
        domain: None,
    }));
    (server, counts, display_tx)
}

async fn local_addr_of(ev: &UnboundedSender<ServerEvent>) -> SocketAddr {
    let (tx, rx) = oneshot::channel();
    ev.send(ServerEvent::GetLocalAddr(tx)).unwrap();
    rx.await.unwrap().unwrap()
}

/// Serve one real client through the full handshake under `policy` and report
/// `(on_accept reached the handler, on_connection_info reached the handler)`.
async fn hooks_reaching_handler_under(policy: server::ConnectionPolicy) -> (bool, bool) {
    let (mut server, counts, _display_tx) = hook_recording_server(policy);
    let ev = server.event_sender().clone();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let server_task = tokio::task::spawn_local(async move {
                let _ = server.run().await;
            });

            let server_addr = local_addr_of(&ev).await;
            let (client, _) = connect_client(server_addr, |client_addr| {
                connector::ClientConnector::new(default_client_config(), client_addr)
            })
            .await;

            // The client is now active, so the server has finished its side of
            // the handshake. Keep the connection open while the hook lands.
            let (accept, info, _) = counts.wait_until(|(_, info, _)| info >= 1).await;

            drop(client);
            server_task.abort();
            (accept >= 1, info >= 1)
        })
        .await
}

#[tokio::test]
async fn connection_handler_hooks_reach_handler_under_queue() {
    let (accept, info) = Box::pin(hooks_reaching_handler_under(server::ConnectionPolicy::Queue)).await;
    assert!(accept && info, "Queue: on_accept={accept}, on_connection_info={info}");
}

#[tokio::test]
async fn connection_handler_hooks_reach_handler_under_preempt() {
    let (accept, info) = Box::pin(hooks_reaching_handler_under(server::ConnectionPolicy::Preempt)).await;
    assert!(accept && info, "Preempt: on_accept={accept}, on_connection_info={info}");
}

#[tokio::test]
async fn connection_handler_hooks_reach_handler_under_reject() {
    let (accept, info) = Box::pin(hooks_reaching_handler_under(server::ConnectionPolicy::Reject)).await;
    assert!(accept && info, "Reject: on_accept={accept}, on_connection_info={info}");
}

/// The interleaving the shared handler exists for: under `Preempt`, a second
/// client's `on_accept` runs through the race's handle while the first session
/// is live, the newcomer takes over, the evicted session's `on_disconnected`
/// still lands, and the newcomer's own `on_connection_info` reaches the same
/// handler.
#[tokio::test]
async fn connection_handler_sees_every_hook_across_a_preemption() {
    let (mut server, counts, _display_tx) = hook_recording_server(server::ConnectionPolicy::Preempt);
    let ev = server.event_sender().clone();

    let local = tokio::task::LocalSet::new();
    Box::pin(local.run_until(async move {
        let server_task = tokio::task::spawn_local(async move {
            let _ = server.run().await;
        });
        let server_addr = local_addr_of(&ev).await;
        let connect = || {
            connect_client(server_addr, |client_addr| {
                connector::ClientConnector::new(default_client_config(), client_addr)
            })
        };

        let (mut first, _) = connect().await;
        assert_eq!(
            counts.wait_until(|(_, info, _)| info >= 1).await,
            (1, 1, 0),
            "first session: (on_accept, on_connection_info, on_disconnected)"
        );

        // The newcomer only finishes its handshake if it takes the session over.
        let (second, _) = tokio::time::timeout(Duration::from_secs(10), connect())
            .await
            .expect("the newcomer must take the session over");

        assert_eq!(
            counts.wait_until(|(_, info, disc)| info >= 2 && disc >= 1).await,
            (2, 2, 1),
            "after the takeover: both accepted, both reported connection info, the evicted one disconnected"
        );

        // And the evicted client's transport is closed, not left open.
        let evicted = tokio::time::timeout(Duration::from_secs(5), async {
            while first.read_pdu().await.is_ok() {}
        })
        .await;
        assert!(evicted.is_ok(), "the evicted client's connection must be closed");

        drop(second);
        server_task.abort();
    }))
    .await;
}

/// Under `Hybrid` the out-of-the-box policy is `Preempt`: with no
/// `with_connection_policy` call, a second client that completes CredSSP takes
/// the session over, and the first client's connection is closed.
#[tokio::test]
async fn the_default_under_hybrid_lets_an_authenticated_newcomer_take_over() {
    const HYBRID_USER: &str = "user";
    const HYBRID_PASSWORD: &str = "password";

    let cert_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-cert.pem");
    let key_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-key.pem");
    let identity = TlsIdentityCtx::init_from_paths(&cert_path, &key_path).expect("failed to init TLS identity");
    let acceptor = identity.make_acceptor().expect("failed to build TLS acceptor");

    // Keep the display sender alive: once it drops, the server closes the session.
    let (_display_tx, display_rx) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_hybrid(acceptor, identity.pub_key.clone())
        .with_input_handler(TestInputHandler)
        .with_display_handler(TestDisplay {
            rx: Arc::new(Mutex::new(display_rx)),
        })
        .build();
    server.set_credentials(Some(server::Credentials {
        username: HYBRID_USER.into(),
        password: HYBRID_PASSWORD.into(),
        domain: None,
    }));
    let ev = server.event_sender().clone();

    let client_config = || connector::Config {
        credentials: connector::Credentials::UsernamePassword {
            username: HYBRID_USER.into(),
            password: HYBRID_PASSWORD.into(),
        },
        ..default_client_config()
    };

    let local = tokio::task::LocalSet::new();
    Box::pin(local.run_until(async move {
        let server_task = tokio::task::spawn_local(async move {
            let _ = server.run().await;
        });
        let server_addr = local_addr_of(&ev).await;
        let connect = || {
            connect_client(server_addr, |client_addr| {
                connector::ClientConnector::new(client_config(), client_addr)
            })
        };

        let (mut first, _) = connect().await;

        // The newcomer can only finish its handshake if the server serves it,
        // which under `Queue` it would not do while `first` is live.
        let (second, _) = tokio::time::timeout(Duration::from_secs(10), connect())
            .await
            .expect("the authenticated newcomer must be served (Preempt), not queued");

        // The incumbent is evicted: its transport ends rather than staying open.
        let evicted = tokio::time::timeout(Duration::from_secs(5), async {
            while first.read_pdu().await.is_ok() {}
        })
        .await;
        assert!(evicted.is_ok(), "the incumbent's connection must be closed");

        drop(second);
        server_task.abort();
    }))
    .await;
}
