use std::f64::consts::TAU;
use std::io::BufRead as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context as _, ensure};
use ironrdp_client::config::{ConfigBuilder, Destination};
use ironrdp_client::h264::H264DecoderFactory;
use ironrdp_client::output_channel::output_channel;
use ironrdp_client::rdp::{RdpClient, RdpInputEvent, RdpInputSender, RdpOutputEvent};
use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_pdu::input::mouse::{MousePdu, PointerFlags};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpeai::client::{AudioPacketSink, RdpeaiCaptureHandler, RdpeaiClient};
use ironrdp_rdpeai::pdu::pcm_format;
use ironrdp_rdpecam::{
    CameraBackend, CameraRedirectionSession, DeviceClient, DeviceDescriptor, EnumerationClient, ErrorCode, MediaFormat,
    MediaType, SampleDelivery, SampleSink, StartStreamInfo, StreamDescription,
};
use ironrdp_rdpei::pdu::{
    PenContact, PenContactFlags, PenEventPdu, PenFrame, TouchContact, TouchContactFlags, TouchEventPdu, TouchFrame,
};
use ironrdp_rdpel::client::LocationClient;
use ironrdp_rdpel::pdu::{BaseLocation3dPdu, Location2dDeltaPdu, LocationPdu};
use ironrdp_rdpewa::pdu::{CborKey, CborValue, decode_all, encode_to_vec};
use ironrdp_rdpewa::{
    DeviceInfo, E_ABORT, E_INVALIDARG, RdpewaClient, RdpewaClientHandler, RdpewaHandlerError, RdpewaResponse,
    RdpewaResponseSender, RdpewaResult, WebAuthnDispatch, WebAuthnOperationRequest, WebAuthnOperationResponse,
    WebAuthnSubcommand,
};
use ironrdp_rdpsnd::client::{Rdpsnd, RdpsndClientHandler};
use ironrdp_rdpsnd::pdu::{AudioFormat, PitchPdu, VolumePdu, WaveFormat};
use sha2::{Digest as _, Sha256};

type SenderSlot = Arc<Mutex<Option<RdpInputSender>>>;

struct Enumeration {
    inner: Arc<Mutex<EnumerationState>>,
}

struct EnumerationState {
    client: EnumerationClient,
    channel_id: Option<u32>,
}

impl_as_any!(Enumeration);

impl DvcProcessor for Enumeration {
    fn channel_name(&self) -> &str {
        ironrdp_rdpecam::ENUMERATION_CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let mut state = self.inner.lock().expect("enumeration state poisoned");
        state.channel_id = Some(channel_id);
        state.client.start(channel_id)
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.inner
            .lock()
            .expect("enumeration state poisoned")
            .client
            .process(channel_id, payload)
    }

    fn close(&mut self, channel_id: u32) {
        let mut state = self.inner.lock().expect("enumeration state poisoned");
        state.client.close(channel_id);
        if state.channel_id == Some(channel_id) {
            state.channel_id = None;
        }
    }
}

impl DvcClientProcessor for Enumeration {}

struct Audio {
    formats: Vec<AudioFormat>,
    amplitude: f64,
    active: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Audio {
    fn new(amplitude: f64) -> Self {
        Self {
            formats: vec![pcm_format(1, 48_000, 16)],
            amplitude,
            active: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }
}

impl RdpeaiCaptureHandler for Audio {
    fn supported_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn open(
        &mut self,
        capture_format: &AudioFormat,
        encode_format: &AudioFormat,
        packet_size: usize,
        mut sink: AudioPacketSink,
    ) -> i32 {
        if capture_format != &self.formats[0] || encode_format != &self.formats[0] || packet_size != 960 {
            eprintln!("Unexpected audio format or packet size");
            return -1;
        }
        self.close();
        self.active.store(true, Ordering::Release);
        let active = Arc::clone(&self.active);
        let amplitude = self.amplitude;
        self.worker = Some(thread::spawn(move || {
            let mut sample_index = 0u64;
            thread::sleep(Duration::from_millis(50));
            while active.load(Ordering::Acquire) {
                let mut packet = Vec::with_capacity(960);
                for _ in 0..480 {
                    let phase = (sample_index as f64) * 440.0 * TAU / 48_000.0;
                    let sample = (phase.sin() * amplitude) as i16;
                    packet.extend_from_slice(&sample.to_le_bytes());
                    sample_index += 1;
                }
                sink(packet);
                thread::sleep(Duration::from_millis(10));
            }
        }));
        eprintln!("Synthetic microphone opened");
        0
    }

    fn set_format(&mut self, encode_format: &AudioFormat, packet_size: usize) -> bool {
        encode_format == &self.formats[0] && packet_size == 960
    }

    fn close(&mut self) {
        self.active.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("microphone worker panicked");
            eprintln!("Synthetic microphone closed");
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.close();
    }
}

struct Camera {
    format: MediaFormat,
    red: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Camera {
    fn new(format: MediaFormat, red: Arc<AtomicBool>) -> Self {
        Self {
            format,
            red,
            active: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }

    fn media_type(&self) -> MediaType {
        let flags = if matches!(self.format, MediaFormat::H264 | MediaFormat::Mjpg) {
            MediaType::DECODING_REQUIRED
        } else {
            0
        };
        MediaType::new(self.format, 320, 240, 30, 1, 1, 1, flags).expect("fixture media type")
    }

    fn frame(&self) -> Vec<u8> {
        match self.format {
            MediaFormat::I420 => {
                let [y, u, v] = if self.red.load(Ordering::Acquire) {
                    [81, 90, 240]
                } else {
                    [145, 54, 34]
                };
                let mut frame = vec![y; 320 * 240];
                frame.extend(vec![u; 320 * 240 / 4]);
                frame.extend(vec![v; 320 * 240 / 4]);
                frame
            }
            MediaFormat::Nv12 => {
                let mut frame = vec![145; 320 * 240];
                frame.extend([54, 34].into_iter().cycle().take(320 * 240 / 2));
                frame
            }
            MediaFormat::Yuy2 => [145, 54, 145, 34].into_iter().cycle().take(320 * 240 * 2).collect(),
            MediaFormat::Mjpg => include_bytes!("rdp_media.jpg").to_vec(),
            MediaFormat::H264 => {
                let y = vec![145u8; 320 * 240];
                let u = vec![54u8; 160 * 120];
                let v = vec![34u8; 160 * 120];
                let yuv = openh264::formats::YUVSlices::new((&y, &u, &v), (320, 240), (320, 160, 160));
                let mut encoder = openh264::encoder::Encoder::new().expect("fixture H.264 encoder");
                encoder.encode(&yuv).expect("fixture H.264 IDR frame").to_vec()
            }
            _ => unreachable!("fixture only selects I420, NV12, YUY2, MJPEG, or H264"),
        }
    }
}

impl CameraBackend for Camera {
    fn activate(&mut self) -> Result<(), ErrorCode> {
        Ok(())
    }

    fn deactivate(&mut self) -> Result<(), ErrorCode> {
        self.stop_streams()
    }

    fn streams(&mut self) -> Result<Vec<StreamDescription>, ErrorCode> {
        Ok(vec![StreamDescription::color(true, false)])
    }

    fn media_types(&mut self, stream_index: u8) -> Result<Vec<MediaType>, ErrorCode> {
        if stream_index != 0 {
            return Err(ErrorCode::InvalidStreamNumber);
        }
        Ok(vec![self.media_type()])
    }

    fn current_media_type(&mut self, stream_index: u8) -> Result<MediaType, ErrorCode> {
        self.media_types(stream_index).map(|types| types[0])
    }

    fn start_streams(&mut self, streams: &[StartStreamInfo], sink: SampleSink) -> Result<(), ErrorCode> {
        if streams
            != [StartStreamInfo {
                stream_index: 0,
                media_type: self.media_type(),
            }]
        {
            return Err(ErrorCode::InvalidMediaType);
        }
        self.stop_streams()?;
        self.active.store(true, Ordering::Release);
        let active = Arc::clone(&self.active);
        let frame = self.frame();
        self.worker = Some(thread::spawn(move || {
            while active.load(Ordering::Acquire) && sink.is_active() {
                if 0 < sink.pending_requests(0) {
                    match sink.send_sample(0, frame.clone()) {
                        SampleDelivery::Sent | SampleDelivery::NotRequested | SampleDelivery::Failed => {}
                        other => {
                            eprintln!("Camera sample rejected: {other:?}");
                            break;
                        }
                    }
                }
                thread::sleep(Duration::from_millis(33));
            }
        }));
        eprintln!("Synthetic camera started");
        Ok(())
    }

    fn stop_streams(&mut self) -> Result<(), ErrorCode> {
        self.active.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("camera worker panicked");
            eprintln!("Synthetic camera stopped");
        }
        Ok(())
    }

    fn shutdown(&mut self) {
        self.stop_streams().expect("camera stopped");
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.shutdown();
    }
}

const WEBAUTHN_AAGUID: [u8; 16] = *b"ironrdp-rdpewa!!";

const PLAYBACK_FORMATS: [AudioFormat; 1] = [AudioFormat {
    format: WaveFormat::PCM,
    n_channels: 2,
    n_samples_per_sec: 48_000,
    n_avg_bytes_per_sec: 48_000 * 4,
    n_block_align: 4,
    bits_per_sample: 16,
    data: None,
}];

#[derive(Debug, Default)]
struct PlaybackSink {
    verified: bool,
}

impl RdpsndClientHandler for PlaybackSink {
    fn get_formats(&self) -> &[AudioFormat] {
        &PLAYBACK_FORMATS
    }

    fn wave(&mut self, format: &AudioFormat, _ts: u32, data: std::borrow::Cow<'_, [u8]>) {
        if self.verified
            || format.n_samples_per_sec != 48_000
            || format.n_channels != 2
            || format.bits_per_sample != 16
            || data.len() < 4
        {
            return;
        }
        let samples = data.chunks_exact(2).map(|pair| {
            let value = i16::from_le_bytes([pair[0], pair[1]]);
            f64::from(value) / 32768.0
        });
        let count = data.len() / 2;
        let rms = (samples.map(|s| s * s).sum::<f64>() / count as f64).sqrt();
        if rms > 0.15 {
            self.verified = true;
            eprintln!("Synthetic RDPSND playback verified (rms={rms:.3})");
        }
    }

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {}
}

struct LocationFixture {
    inner: LocationClient,
}

impl_as_any!(LocationFixture);

impl DvcProcessor for LocationFixture {
    fn channel_name(&self) -> &str {
        ironrdp_rdpel::CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.inner.start(channel_id)
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let was_ready = self.inner.ready();
        let mut responses = self.inner.process(channel_id, payload)?;
        if !was_ready && self.inner.ready() {
            let base =
                BaseLocation3dPdu::coordinates(45.5, -73.5, 42).map_err(|error| ironrdp_pdu::encode_err!(error))?;
            let delta =
                Location2dDeltaPdu::coordinates(-0.125, -0.125).map_err(|error| ironrdp_pdu::encode_err!(error))?;
            responses.push(Box::new(LocationPdu::BaseLocation3d(base)));
            responses.push(Box::new(LocationPdu::Location2dDelta(delta)));
            eprintln!("Synthetic RDPLOCATION coordinates sent");
        }
        Ok(responses)
    }

    fn close(&mut self, channel_id: u32) {
        self.inner.close(channel_id);
    }
}

impl DvcClientProcessor for LocationFixture {}
const P256_X: [u8; 32] = [
    0x6B, 0x17, 0xD1, 0xF2, 0xE1, 0x2C, 0x42, 0x47, 0xF8, 0xBC, 0xE6, 0xE5, 0x63, 0xA4, 0x40, 0xF2, 0x77, 0x03, 0x7D,
    0x81, 0x2D, 0xEB, 0x33, 0xA0, 0xF4, 0xA1, 0x39, 0x45, 0xD8, 0x98, 0xC2, 0x96,
];
const P256_Y: [u8; 32] = [
    0x4F, 0xE3, 0x42, 0xE2, 0xFE, 0x1A, 0x7F, 0x9B, 0x8E, 0xE7, 0xEB, 0x4A, 0x7C, 0x0F, 0x9E, 0x16, 0x2B, 0xCE, 0x33,
    0x57, 0x6B, 0x31, 0x5E, 0xCE, 0xCB, 0xB6, 0x40, 0x68, 0x37, 0xBF, 0x51, 0xF5,
];

#[derive(Default)]
struct WebAuthnState {
    pending_cancel: Mutex<Option<(Vec<u8>, RdpewaResponseSender)>>,
}

struct WebAuthn {
    state: Arc<WebAuthnState>,
    aaguid: [u8; 16],
}

impl RdpewaClientHandler for WebAuthn {
    fn api_version(&mut self) -> RdpewaResult<u32> {
        Ok(4)
    }

    fn is_uvpaa(&mut self) -> RdpewaResult<bool> {
        eprintln!("Synthetic WebAuthn IUVPAA queried");
        Ok(true)
    }

    fn cancel_current_operation(&mut self, cancellation_id: &[u8]) -> RdpewaResult<()> {
        if cancellation_id.len() != 16 {
            return Err(RdpewaHandlerError::new(E_INVALIDARG, "invalid cancellation GUID"));
        }
        let mut pending = self.state.pending_cancel.lock().expect("webauthn state poisoned");
        if let Some((expected, mut reply)) = pending.take()
            && expected == cancellation_id
        {
            eprintln!("Synthetic WebAuthn cancelled");
            reply.send(RdpewaResponse::from_hresult(E_ABORT));
        }
        Ok(())
    }

    fn begin_webauthn(
        &mut self,
        request: WebAuthnOperationRequest,
        reply: RdpewaResponseSender,
    ) -> RdpewaResult<WebAuthnDispatch> {
        if request.client_data_json.is_empty() {
            return Err(RdpewaHandlerError::new(E_INVALIDARG, "missing clientDataJSON"));
        }
        let rp_id = request
            .rp_id
            .as_deref()
            .filter(|rp| *rp == "localhost")
            .ok_or_else(|| RdpewaHandlerError::new(E_INVALIDARG, "unexpected rpId"))?;
        if !request
            .para
            .ui_origin
            .as_deref()
            .is_some_and(|origin| origin.starts_with("http://localhost:"))
        {
            return Err(RdpewaHandlerError::new(E_INVALIDARG, "unexpected uiOrigin"));
        }
        match request.subcommand {
            WebAuthnSubcommand::MakeCredential => {
                let ctap = decode_all(&request.ctap_cbor)
                    .and_then(|value| value.into_map())
                    .map_err(|_| RdpewaHandlerError::new(E_INVALIDARG, "invalid CTAP CBOR"))?;
                let hash = ctap
                    .get(&CborKey::Int(1))
                    .and_then(|value| value.as_bytes().ok())
                    .ok_or_else(|| RdpewaHandlerError::new(E_INVALIDARG, "missing clientDataHash"))?;
                if hash != Sha256::digest(&request.client_data_json).as_slice() {
                    return Err(RdpewaHandlerError::new(E_INVALIDARG, "clientDataJSON hash mismatch"));
                }
                let cancel_probe = ctap
                    .get(&CborKey::Int(3))
                    .and_then(|value| value.as_map().ok())
                    .and_then(|user| user.get(&CborKey::text("id")))
                    .and_then(|id| id.as_bytes().ok())
                    == Some(&[5, 6, 7, 8][..]);
                if cancel_probe {
                    let cancel_id = request
                        .para
                        .cancellation_id
                        .clone()
                        .ok_or_else(|| RdpewaHandlerError::new(E_INVALIDARG, "missing cancellationId"))?;
                    *self.state.pending_cancel.lock().expect("webauthn state poisoned") = Some((cancel_id, reply));
                    eprintln!("Synthetic WebAuthn waiting for cancel");
                    return Ok(WebAuthnDispatch::Async);
                }
                let response = build_make_credential_ctap(rp_id, &self.aaguid)?;
                eprintln!("Synthetic WebAuthn credential created");
                Ok(WebAuthnDispatch::Sync(WebAuthnOperationResponse {
                    device_info: DeviceInfo {
                        aa_guid: self.aaguid,
                        resident_key: Some(request.para.require_resident_key),
                        ..DeviceInfo::default()
                    },
                    status: 0,
                    response,
                }))
            }
            WebAuthnSubcommand::GetAssertion => {
                let ctap = decode_all(&request.ctap_cbor)
                    .and_then(|value| value.into_map())
                    .map_err(|_| RdpewaHandlerError::new(E_INVALIDARG, "invalid GetAssertion CBOR"))?;
                let hash = ctap
                    .get(&CborKey::Int(2))
                    .and_then(|value| value.as_bytes().ok())
                    .ok_or_else(|| RdpewaHandlerError::new(E_INVALIDARG, "missing GetAssertion clientDataHash"))?;
                if hash != Sha256::digest(&request.client_data_json).as_slice() {
                    return Err(RdpewaHandlerError::new(
                        E_INVALIDARG,
                        "GetAssertion clientDataJSON hash mismatch",
                    ));
                }
                eprintln!("Synthetic WebAuthn assertion checked");
                Ok(WebAuthnDispatch::Sync(WebAuthnOperationResponse {
                    device_info: DeviceInfo::default(),
                    status: 0x2E,
                    response: vec![0x2E],
                }))
            }
        }
    }
}

fn build_make_credential_ctap(rp_id: &str, aaguid: &[u8; 16]) -> RdpewaResult<Vec<u8>> {
    let credential_id = &Sha256::digest(rp_id.as_bytes())[..16];
    let mut auth_data = Sha256::digest(rp_id.as_bytes()).to_vec();
    auth_data.extend_from_slice(&[0x45, 0, 0, 0, 0]);
    auth_data.extend_from_slice(aaguid);
    auth_data.extend_from_slice(&16u16.to_be_bytes());
    auth_data.extend_from_slice(credential_id);
    let mut cose_key = std::collections::BTreeMap::new();
    cose_key.insert(CborKey::Int(1), CborValue::Unsigned(2));
    cose_key.insert(CborKey::Int(3), CborValue::Negative(-7));
    cose_key.insert(CborKey::Int(-1), CborValue::Unsigned(1));
    cose_key.insert(CborKey::Int(-2), CborValue::Bytes(P256_X.to_vec()));
    cose_key.insert(CborKey::Int(-3), CborValue::Bytes(P256_Y.to_vec()));
    auth_data.extend_from_slice(
        &encode_to_vec(&CborValue::Map(cose_key)).map_err(|_| RdpewaHandlerError::fail("encode COSE key"))?,
    );
    let mut ctap_map = std::collections::BTreeMap::new();
    ctap_map.insert(CborKey::Int(1), CborValue::Text("none".to_owned()));
    ctap_map.insert(CborKey::Int(2), CborValue::Bytes(auth_data));
    ctap_map.insert(CborKey::Int(3), CborValue::Map(std::collections::BTreeMap::new()));
    let mut out = vec![0x00];
    out.extend_from_slice(
        &encode_to_vec(&CborValue::Map(ctap_map)).map_err(|_| RdpewaHandlerError::fail("encode attestation"))?,
    );
    Ok(out)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().context("missing RDP port")?.parse()?;
    let certificate = std::fs::read(args.next().context("missing DER certificate")?)?;
    ensure!(args.next().is_none(), "unexpected arguments");
    let expected_endpoint = format!("127.0.0.1:{port}");
    let provide_devices = std::env::var("RDP_MEDIA_FIXTURE_DEVICES").as_deref() != Ok("none");
    let red = Arc::new(AtomicBool::new(
        std::env::var("RDP_MEDIA_FIXTURE_RECONNECTED").as_deref() == Ok("1"),
    ));
    let format = match std::env::var("RDP_MEDIA_FIXTURE_FORMAT").as_deref() {
        Ok("i420") => MediaFormat::I420,
        Ok("nv12") => MediaFormat::Nv12,
        Ok("yuy2") => MediaFormat::Yuy2,
        Ok("mjpeg") => MediaFormat::Mjpg,
        Ok("h264") => MediaFormat::H264,
        _ => anyhow::bail!("invalid fixture camera format"),
    };
    ensure!(
        !red.load(Ordering::Acquire) || format == MediaFormat::I420,
        "reconnected fixture requires I420"
    );

    let sender_slot: SenderSlot = Arc::new(Mutex::new(None));
    let audio_sender = Arc::clone(&sender_slot);
    let camera_sender = Arc::clone(&sender_slot);
    let webauthn_sender = Arc::clone(&sender_slot);
    let webauthn_state = Arc::new(WebAuthnState::default());
    let session = CameraRedirectionSession::new();
    let camera = session.select_device(DeviceDescriptor::new(
        "Synthetic camera".into(),
        "RDCamera_Device_0".into(),
    )?)?;
    let enumeration = Arc::new(Mutex::new(EnumerationState {
        client: session.enumeration_client(vec![camera.clone()])?,
        channel_id: None,
    }));
    let enumeration_factory = Arc::clone(&enumeration);
    let hotplug_camera = camera.clone();
    let hotplug_red = Arc::clone(&red);
    let initial_red = red.load(Ordering::Acquire);
    let config = ConfigBuilder::new()
        .with_destination(Destination::from_parts("127.0.0.1", port))
        .with_username("fixture")
        .with_password("fixture")
        .with_credssp(false)
        .with_client_build(1)
        .with_client_dir("C:\\")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_client_name("fixture")
        .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
        .with_certificate_validation_callback(Arc::new(move |actual, endpoint, _| {
            endpoint == expected_endpoint && actual == certificate
        }))
        .with_h264_decoder(Some(H264DecoderFactory::openh264()))
        .with_static_channel(move |_| provide_devices.then(|| Rdpsnd::new(Box::new(PlaybackSink::default()))))
        .with_dvc(move |_| {
            provide_devices.then(|| LocationFixture {
                inner: LocationClient::new(),
            })
        })
        .with_dvc(move |_| {
            if !provide_devices {
                return None;
            }
            let sender = Arc::clone(&audio_sender);
            Some(RdpeaiClient::new(
                Box::new(Audio::new(if initial_red { 8_000.0 } else { 16_000.0 })),
                Box::new(move |channel_id, messages| {
                    let sender = sender.lock().expect("sender lock poisoned");
                    sender
                        .as_ref()
                        .expect("client sender initialized")
                        .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                        .map_err(|_| pdu_other_err!("enqueue audio DVC messages"))
                }),
            ))
        })
        .with_dvc(move |_| {
            provide_devices.then(|| Enumeration {
                inner: Arc::clone(&enumeration_factory),
            })
        })
        .with_dvc(move |_| {
            if !provide_devices {
                return None;
            }
            let sender = Arc::clone(&camera_sender);
            Some(DeviceClient::new(
                &camera,
                Camera::new(format, Arc::clone(&red)),
                Arc::new(move |channel_id, messages| {
                    let sender = sender.lock().expect("sender lock poisoned");
                    sender
                        .as_ref()
                        .expect("client sender initialized")
                        .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                        .map_err(|_| pdu_other_err!("enqueue camera DVC messages"))
                }),
            ))
        })
        .with_dvc(move |_| {
            if !provide_devices {
                return None;
            }
            let sender = Arc::clone(&webauthn_sender);
            let state = Arc::clone(&webauthn_state);
            let aaguid = if initial_red {
                *b"ironrdp-reconn!!"
            } else {
                WEBAUTHN_AAGUID
            };
            Some(
                RdpewaClient::new(Box::new(WebAuthn { state, aaguid })).with_write_callback(
                    move |channel_id, messages| {
                        let sender = sender.lock().expect("sender lock poisoned");
                        sender
                            .as_ref()
                            .expect("client sender initialized")
                            .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                            .map_err(|_| pdu_other_err!("enqueue WebAuthn DVC messages"))
                    },
                ),
            )
        })
        .build()
        .context("build RDP client config")?;
    let (output_tx, mut output_rx) = output_channel(64);
    let client = RdpClient::new(config, output_tx);
    let input = client.input_sender();
    *sender_slot.lock().expect("sender lock poisoned") = Some(input.clone());
    let (command_tx, mut command_rx) = tokio::sync::mpsc::unbounded_channel();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if command_tx.send(line).is_err() {
                return;
            }
        }
    });
    tokio::task::LocalSet::new()
        .run_until(async move {
            let client_task = tokio::task::spawn_local(client.run());
            let deadline = tokio::time::sleep(Duration::from_secs(90));
            tokio::pin!(deadline);
            let mut connected = false;
            let mut rdpei_ready = false;
            let mut rdpei_pen_ready = false;
            let mut pointer_received = false;
            let mut egfx_frame_received = false;
            let mut hover_toggle = false;
            let mut touch_attempts = 0u8;
            let mut interact_tick = tokio::time::interval(Duration::from_millis(250));
            interact_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut closing = false;
            loop {
                tokio::select! {
                    () = &mut deadline => {
                        input.request_close();
                        anyhow::bail!("RDP client timed out");
                    }
                    _ = interact_tick.tick(), if connected && !closing && (!pointer_received || (rdpei_ready && touch_attempts < 8)) => {
                        if !pointer_received {
                            hover_toggle = !hover_toggle;
                            let x_position = if hover_toggle { 60 } else { 200 };
                            let move_event = FastPathInputEvent::MouseEvent(MousePdu {
                                flags: PointerFlags::MOVE,
                                number_of_wheel_rotation_units: 0,
                                x_position,
                                y_position: 120,
                            });
                            let _ = input.try_send(RdpInputEvent::FastPath([move_event].into_iter().collect()));
                        }
                        if rdpei_ready && touch_attempts < 8 {
                            touch_attempts += 1;
                            let down = TouchContact::new(
                                0,
                                60,
                                120,
                                TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT,
                            );
                            let up = TouchContact::new(0, 60, 120, TouchContactFlags::UP);
                            let _ = input.try_send(RdpInputEvent::Touch(TouchEventPdu::new(
                                0,
                                vec![TouchFrame {
                                    frame_offset: 0,
                                    contacts: vec![down],
                                }],
                            )));
                            let _ = input.try_send(RdpInputEvent::Touch(TouchEventPdu::new(
                                10,
                                vec![TouchFrame {
                                    frame_offset: 0,
                                    contacts: vec![up],
                                }],
                            )));
                            if rdpei_pen_ready {
                                let pen_down = PenContact::new(
                                    0,
                                    60,
                                    120,
                                    PenContactFlags::DOWN | PenContactFlags::INRANGE | PenContactFlags::INCONTACT,
                                )
                                .with_pressure(512);
                                let pen_up = PenContact::new(0, 60, 120, PenContactFlags::UP);
                                let _ = input.try_send(RdpInputEvent::Pen(PenEventPdu::new(
                                    15,
                                    vec![PenFrame {
                                        frame_offset: 0,
                                        contacts: vec![pen_down],
                                    }],
                                )));
                                let _ = input.try_send(RdpInputEvent::Pen(PenEventPdu::new(
                                    25,
                                    vec![PenFrame {
                                        frame_offset: 0,
                                        contacts: vec![pen_up],
                                    }],
                                )));
                            }
                        }
                    }
                    command = command_rx.recv(), if !closing => {
                        match command {
                            None => {
                                input.request_close();
                                closing = true;
                            }
                            Some(line) => {
                                let line = line.context("read fixture input")?;
                                let mut state = enumeration.lock().expect("enumeration state poisoned");
                                let channel_id = state.channel_id.context("camera enumeration channel is not open")?;
                                let messages = match line.as_str() {
                                    "replace" => {
                                        let mut messages =
                                            state.client.remove_device(&hotplug_camera.descriptor().channel_name)?;
                                        hotplug_red.store(true, Ordering::Release);
                                        messages.extend(state.client.add_device(hotplug_camera.clone())?);
                                        messages
                                    }
                                    _ => anyhow::bail!("unknown fixture command"),
                                };
                                ensure!(!messages.is_empty(), "camera enumeration is not ready");
                                input.try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                                    .map_err(|error| anyhow::anyhow!("send camera hotplug messages: {error}"))?;
                                eprintln!("Synthetic camera {line} sent");
                            }
                        }
                    }
                    event = output_rx.recv() => {
                        match event.context("RDP client event channel closed")? {
                            RdpOutputEvent::Connected => {
                                connected = true;
                                eprintln!("RDP client connected");
                            }
                            RdpOutputEvent::RdpeiInputState(state) => {
                                rdpei_ready = state.touch && !state.suspended;
                                rdpei_pen_ready = state.pen && !state.suspended;
                            }
                            RdpOutputEvent::Image { width, height, .. } => {
                                if !egfx_frame_received && width.get() >= 320 && height.get() >= 240 {
                                    egfx_frame_received = true;
                                    eprintln!("Synthetic desktop frame received ({}x{})", width.get(), height.get());
                                }
                            }
                            RdpOutputEvent::PointerBitmap(pointer) => {
                                if !pointer_received && pointer.width == 24 && pointer.height == 24 {
                                    pointer_received = true;
                                    eprintln!("Synthetic pointer bitmap received");
                                }
                            }
                            RdpOutputEvent::ConnectionFailure(error) => {
                                anyhow::bail!("RDP connection failed: {error:?}");
                            }
                            RdpOutputEvent::Terminated(result) => {
                                let reason = result.context("RDP client terminated with an error")?;
                                eprintln!("RDP client terminated: {reason:?}");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
            input.request_close();
            client_task.await.context("RDP client task failed")?;
            ensure!(connected, "RDP client never connected");
            Ok(())
        })
        .await
}
