// FIXME: tests in this module can probably be rewritten to be much shorter using the ironrdp-client crate.

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use anyhow::Result;
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
use ironrdp::pdu::{self, gcc};
use ironrdp::server::{
    self, Acceptor, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, RdpServerInputHandler, ServerEvent, StaticChannelFactory, TlsIdentityCtx,
};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{self, ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp::svc::{StaticChannelSet, SvcMessage, SvcProcessor, SvcServerProcessor};
use ironrdp_async::{Framed, FramedWrite as _};
use ironrdp_bulk::{BulkCompressor, CompressionType as BulkCompressionType, flags as bulk_flags};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    Capabilities, CoreCapability, CoreCapabilityKind, DeviceCreateResponse, DeviceIoRequest, DeviceIoResponse,
    MajorFunction, MinorFunction, NtStatus, ServerDeviceAnnounceResponse, ServerDriveIoRequest, VERSION_MAJOR,
    VERSION_MINOR_12, VersionAndIdPdu, VersionAndIdPduKind,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::{Rdpdr, RdpdrBackend, RdpdrBackendFactory, RdpdrBackendProduct, RdpdrDrive};
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
                                static_channel_chunk_size,
                                ..
                            } = connection_activation.connection_activation_state()
                            {
                                debug!(?desktop_size, "Deactivation-Reactivation Sequence completed");
                                // Update image size with the new desktop size.
                                // image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                                // Update the active stage with the new channel IDs and pointer settings.
                                assert!(stage.reactivate(
                                    connection_activation.io_channel_id(),
                                    connection_activation.user_channel_id(),
                                    share_id,
                                    enable_server_pointer,
                                    pointer_software_rendering,
                                    static_channel_chunk_size,
                                ));
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

#[tokio::test]
async fn test_echo_virtual_channel_end_to_end() {
    let payload = b"ironrdp echo e2e".to_vec();
    let echo_payload = payload.clone();

    client_server_with_connector(
        default_client_config(),
        Vec::new(),
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
async fn rdpdr_static_channel_announces_a_drive_and_completes_an_unsupported_create() {
    let fixture = RdpdrFixtureFactory::new(RdpdrFixtureOperation::UnsupportedCreate);
    let fixture_state = fixture.state();

    client_server_with_connector(
        default_client_config(),
        vec![Box::new(fixture)],
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
    ) -> pdu::PduResult<()> {
        Ok(())
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
        remote_application_mode: false,
        rail_support_level: pdu::rdp::capability_sets::RailSupportLevel::SUPPORTED,
    }
}
