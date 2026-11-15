use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcMessage, DvcMessageBatch, DvcProcessor, DvcServerProcessor, encode_dvc_messages};
use ironrdp_pdu::PduResult;
use ironrdp_rdpecam::{
    DesiredCameraFormat, DeviceDescriptor, DeviceServer, EnumerationServer, MediaFormat, MediaType, ProtocolVersion,
};
use ironrdp_server::{ServerEvent, ServerEventSender};
use ironrdp_svc::ChannelFlags;
use tokio::sync::{Semaphore, mpsc::UnboundedSender, oneshot};

use crate::rdp::ChannelGuard;
use crate::remote_devices::{PixelFormat, RemoteMedia};

// UNTESTED: Match `kMaxDimension` in `server/cef/patches/remote-devices-chromium.patch`.
// A mismatch makes CEF reject selected frames, so verify both limits when changing either.
const MAX_CEF_CAMERA_DIMENSION: u32 = 4096;
const MAX_ENUMERATED_CAMERAS: usize = 32;

#[derive(Clone)]
pub(crate) struct CameraInput {
    shared: Arc<Mutex<Shared>>,
    media: RemoteMedia,
    dynamic_write_permits: Arc<Semaphore>,
}

#[derive(Default)]
struct Shared {
    events: Option<UnboundedSender<ServerEvent>>,
    connected: bool,
    connection_token: Arc<AtomicBool>,
    active: bool,
    generation: u64,
    enumeration_requested: bool,
    enumeration_channel_id: Option<u32>,
    enumeration_token: Option<ChannelGuard>,
    version: Option<ProtocolVersion>,
    devices: Vec<DeviceDescriptor>,
    device_requested: Option<Arc<str>>,
    device_channel: Option<(u32, String)>,
    device_token: Option<ChannelGuard>,
    device_server: Option<Weak<Mutex<DeviceServer>>>,
    stop_completion: Option<oneshot::Receiver<()>>,
    failed_device: Option<String>,
}

impl CameraInput {
    pub(crate) fn new(media: RemoteMedia) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared::default())),
            media,
            dynamic_write_permits: Arc::new(Semaphore::new(1)),
        }
    }

    pub(crate) fn connected(&self) {
        let mut shared = self.shared.lock().expect("camera-input state poisoned");
        if !shared.connected {
            shared.connection_token = Arc::new(AtomicBool::new(true));
            shared.connected = true;
        }
    }

    pub(crate) fn disconnected(&self) {
        let mut shared = self.shared.lock().expect("camera-input state poisoned");
        shared.connection_token.store(false, Ordering::Release);
        shared.connected = false;
        shared.generation = shared.generation.wrapping_add(1);
        shared.enumeration_requested = false;
        shared.enumeration_channel_id = None;
        shared.enumeration_token.take().inspect(ChannelGuard::invalidate);
        shared.version = None;
        shared.devices.clear();
        shared.device_requested = None;
        shared.device_channel = None;
        shared.device_server = None;
        shared.stop_completion = None;
        shared.failed_device = None;
        shared.device_token.take().inspect(ChannelGuard::invalidate);
    }

    pub(crate) fn tick(&self) {
        let requested_format = self.media.camera_format();
        let (events, mut queued, create_enumeration, create_device, stop_device) = {
            let mut shared = self.shared.lock().expect("camera-input state poisoned");
            let Some(events) = shared.events.clone() else {
                return;
            };
            let mut queued = Vec::new();
            let mut create_enumeration = None;
            let mut create_device = None;
            let mut stop_device = None;
            if !shared.connected {
                if requested_format.is_none() && shared.active {
                    shared.active = false;
                    shared.generation = shared.generation.wrapping_add(1);
                }
                return;
            }
            if let Some(mut completion) = shared.stop_completion.take() {
                match completion.try_recv() {
                    Ok(()) => {}
                    Err(oneshot::error::TryRecvError::Empty) => shared.stop_completion = Some(completion),
                    Err(oneshot::error::TryRecvError::Closed) => {
                        tracing::warn!("Camera shutdown message was not written; closing the dynamic channel");
                        if let Some((channel_id, _)) = shared.device_channel.take()
                            && let Some(token) = shared.device_token.take()
                        {
                            queued.push(token.close(channel_id));
                        }
                        shared.device_server = None;
                        shared.device_requested = None;
                    }
                }
            }
            if let Some(format) = requested_format {
                if !shared.active {
                    shared.active = true;
                    shared.generation = shared.generation.wrapping_add(1);
                }
                if shared.enumeration_channel_id.is_none() && !shared.enumeration_requested {
                    shared.enumeration_requested = true;
                    create_enumeration = Some(shared.generation);
                }
                let selected = shared.devices.first().cloned();
                let current_name = shared.device_channel.as_ref().map(|(_, name)| name.as_str());
                if selected.as_ref().map(|device| device.channel_name.as_str()) != current_name
                    && let Some((channel_id, _)) = shared.device_channel.take()
                    && let Some(token) = shared.device_token.take()
                {
                    queued.push(token.close(channel_id));
                    shared.device_requested = None;
                    shared.device_server = None;
                }
                if shared.version.is_some()
                    && let Some(device) = selected
                    && shared.device_channel.is_none()
                    && shared.device_requested.is_none()
                    && shared.failed_device.as_deref() != Some(device.channel_name.as_str())
                {
                    // A distinct allocation identifies each request, even if a camera name is reused.
                    let request: Arc<str> = Arc::from(device.channel_name.as_str());
                    shared.device_requested = Some(Arc::clone(&request));
                    let version = shared.version.expect("version checked");
                    create_device = Some((device, version, format, shared.generation, request));
                }
            } else {
                if shared.active {
                    shared.active = false;
                    shared.generation = shared.generation.wrapping_add(1);
                    shared.failed_device = None;
                }
                if let Some((channel_id, _)) = shared.device_channel.as_ref()
                    && let (Some(server), Some(token)) = (
                        shared.device_server.as_ref().and_then(Weak::upgrade),
                        shared.device_token.as_ref(),
                    )
                {
                    stop_device = Some((server, *channel_id, token.clone()));
                } else if let Some((channel_id, _)) = shared.device_channel.take()
                    && let Some(token) = shared.device_token.take()
                {
                    queued.push(token.close(channel_id));
                }
                if let Some(channel_id) = shared.enumeration_channel_id.take()
                    && let Some(token) = shared.enumeration_token.take()
                {
                    queued.push(token.close(channel_id));
                }
                shared.enumeration_requested = false;
                shared.version = None;
                shared.devices.clear();
                shared.device_requested = None;
            }
            (events, queued, create_enumeration, create_device, stop_device)
        };
        if let Some(generation) = create_enumeration {
            queued.push(self.create_enumeration_event(generation));
        }
        if let Some((device, version, format, generation, request)) = create_device {
            queued.push(self.create_device_event(device, version, format, generation, request));
        }
        if let Some((server, channel_id, token)) = stop_device {
            match stop_device_event(
                &server,
                channel_id,
                token.clone(),
                Arc::clone(&self.dynamic_write_permits),
            ) {
                Ok(Some((event, completion))) => {
                    self.shared.lock().expect("camera-input state poisoned").stop_completion = Some(completion);
                    queued.push(event);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "Failed to prepare camera shutdown message");
                    queued.push(token.close(channel_id));
                }
            }
        }
        for event in queued {
            if let Err(error) = events.send(event) {
                tracing::debug!(%error, "RDP server stopped before a camera event was sent");
                self.disconnected();
                break;
            }
        }
    }

    fn create_enumeration_event(&self, generation: u64) -> ServerEvent {
        let shared = Arc::clone(&self.shared);
        let mut unstarted = UnstartedCameraChannel {
            shared: Arc::clone(&shared),
            generation,
            request: None,
            armed: true,
        };
        ServerEvent::CreateDynamicChannel(Box::new(move |channel_id| {
            unstarted.disarm();
            let (token, current) = {
                let mut state = shared.lock().expect("camera-input state poisoned");
                let token = ChannelGuard::new(&state.connection_token);
                if state.connected && state.active && state.generation == generation && state.enumeration_requested {
                    state.enumeration_channel_id = Some(channel_id);
                    state.enumeration_token = Some(token.clone());
                    (token, true)
                } else {
                    (token, false)
                }
            };
            let added = Arc::clone(&shared);
            let removed = Arc::clone(&shared);
            let processor = EnumerationProcessor {
                inner: EnumerationServer::new(
                    Box::new(move |version, device| {
                        let mut state = added.lock().expect("camera-input state poisoned");
                        if state.generation == generation
                            && state.enumeration_channel_id == Some(channel_id)
                            && state.devices.len() < MAX_ENUMERATED_CAMERAS
                            && !state
                                .devices
                                .iter()
                                .any(|existing| existing.channel_name == device.channel_name)
                        {
                            state.version = Some(version);
                            state.devices.push(device);
                        }
                    }),
                    Box::new(move |version, channel_name| {
                        let close = {
                            let mut state = removed.lock().expect("camera-input state poisoned");
                            if state.generation != generation
                                || state.enumeration_channel_id != Some(channel_id)
                                || state.version != Some(version)
                            {
                                return;
                            }
                            state.devices.retain(|device| device.channel_name != channel_name);
                            if state
                                .device_requested
                                .as_ref()
                                .is_some_and(|requested| requested.as_ref() == channel_name)
                            {
                                state.device_requested = None;
                            }
                            if state.failed_device.as_deref() == Some(channel_name.as_str()) {
                                state.failed_device = None;
                            }
                            if state
                                .device_channel
                                .as_ref()
                                .is_some_and(|(_, current)| current == &channel_name)
                                && let Some((id, _)) = state.device_channel.take()
                                && let Some(token) = state.device_token.take()
                            {
                                state.device_server = None;
                                state.events.clone().map(|events| (events, token.close(id)))
                            } else {
                                None
                            }
                        };
                        if let Some((events, close)) = close
                            && let Err(error) = events.send(close)
                        {
                            tracing::debug!(%error, "RDP server stopped before a removed camera channel was closed");
                        }
                    }),
                ),
                shared: Arc::clone(&shared),
                token: token.clone(),
                generation,
            };
            if !current {
                let events = shared.lock().expect("camera-input state poisoned").events.clone();
                if let Some(events) = events {
                    let _ = events.send(token.close(channel_id));
                }
            }
            Box::new(processor)
        }))
    }

    fn create_device_event(
        &self,
        device: DeviceDescriptor,
        version: ProtocolVersion,
        desired: crate::remote_devices::CameraFormat,
        generation: u64,
        request: Arc<str>,
    ) -> ServerEvent {
        let shared = Arc::clone(&self.shared);
        let media = self.media;
        let mut unstarted = UnstartedCameraChannel {
            shared: Arc::clone(&shared),
            generation,
            request: Some(Arc::clone(&request)),
            armed: true,
        };
        ServerEvent::CreateDynamicChannel(Box::new(move |channel_id| {
            unstarted.disarm();
            let (token, current, events) = {
                let mut state = shared.lock().expect("camera-input state poisoned");
                let token = ChannelGuard::new(&state.connection_token);
                let current = state.connected
                    && state.active
                    && state.generation == generation
                    && state
                        .device_requested
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &request))
                    && state.device_channel.is_none();
                if current {
                    state.device_channel = Some((channel_id, device.channel_name.clone()));
                    state.device_token = Some(token.clone());
                }
                (token, current, state.events.clone())
            };
            let started = Instant::now();
            let frame_shared = Arc::clone(&shared);
            let mut h264_decoder = match openh264::decoder::Decoder::new() {
                Ok(decoder) => Some(decoder),
                Err(error) => {
                    tracing::warn!(%error, "Failed to initialize OpenH264 camera decoder; H.264 streams disabled");
                    None
                }
            };
            let allow_h264 = h264_decoder.is_some();
            let mut packed_i420 = Vec::new();
            let on_frame = Box::new(move |media_type: MediaType, sample: &[u8]| {
                {
                    let state = frame_shared.lock().expect("camera-input state poisoned");
                    if !state.connected
                        || !state.active
                        || state.generation != generation
                        || state.device_channel.as_ref().is_none_or(|(id, _)| *id != channel_id)
                    {
                        return;
                    }
                }
                if media_type.format == MediaFormat::H264 {
                    let Some(decoder) = h264_decoder.as_mut() else {
                        tracing::warn!("Dropping H.264 camera frame without an initialized decoder");
                        return;
                    };
                    match decode_h264_to_i420(decoder, sample, &mut packed_i420) {
                        Ok(Some((width, height))) => {
                            let still_active = {
                                let state = frame_shared.lock().expect("camera-input state poisoned");
                                state.connected
                                    && state.active
                                    && state.generation == generation
                                    && state.device_channel.as_ref().is_some_and(|(id, _)| *id == channel_id)
                            };
                            if still_active
                                && let Err(error) = media.push_camera_frame(
                                    PixelFormat::I420,
                                    width,
                                    height,
                                    &packed_i420,
                                    started.elapsed(),
                                )
                            {
                                tracing::warn!(%error, "CEF rejected a decoded H.264 camera frame");
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            tracing::warn!(%error, "Dropping malformed H.264 RDPECAM camera sample");
                        }
                    }
                    return;
                }
                let Some(format) = pixel_format(media_type.format) else {
                    tracing::warn!(?media_type.format, "Dropping unsupported RDPECAM media format");
                    return;
                };
                if let Err(error) =
                    media.push_camera_frame(format, media_type.width, media_type.height, sample, started.elapsed())
                {
                    tracing::warn!(%error, "CEF rejected an RDPECAM camera frame");
                }
            });
            let failure_shared = Arc::clone(&shared);
            let failure_token = token.clone();
            let failure_name = device.channel_name.clone();
            let on_failure = Box::new(move || {
                let events = {
                    let mut state = failure_shared.lock().expect("camera-input state poisoned");
                    if state.generation == generation
                        && state.device_channel.as_ref().is_some_and(|(id, _)| *id == channel_id)
                    {
                        state.failed_device = Some(failure_name.clone());
                    }
                    state.events.clone()
                };
                if let Some(events) = events
                    && let Err(error) = events.send(failure_token.close(channel_id))
                {
                    tracing::debug!(%error, "RDP server stopped before a failed camera channel was closed");
                }
            });
            let processor = DeviceProcessor {
                inner: Arc::new(Mutex::new(
                    DeviceServer::with_failure_handler(
                        &device,
                        version,
                        DesiredCameraFormat {
                            width: desired.width,
                            height: desired.height,
                            frame_rate: desired.frame_rate,
                            max_width: MAX_CEF_CAMERA_DIMENSION,
                            max_height: MAX_CEF_CAMERA_DIMENSION,
                        },
                        on_frame,
                        on_failure,
                    )
                    .with_h264(allow_h264),
                )),
                channel_name: device.channel_name.clone(),
                shared: Arc::clone(&shared),
                token: token.clone(),
                generation,
            };
            if current {
                let mut state = shared.lock().expect("camera-input state poisoned");
                if state.generation == generation
                    && state.device_channel.as_ref().is_some_and(|(id, _)| *id == channel_id)
                {
                    state.device_server = Some(Arc::downgrade(&processor.inner));
                }
            }
            if !current {
                if let Some(events) = events {
                    let _ = events.send(token.close(channel_id));
                }
            }
            Box::new(processor)
        }))
    }
}

impl ServerEventSender for CameraInput {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.shared.lock().expect("camera-input state poisoned").events = Some(sender);
    }
}

struct EnumerationProcessor {
    inner: EnumerationServer,
    shared: Arc<Mutex<Shared>>,
    token: ChannelGuard,
    generation: u64,
}

impl_as_any!(EnumerationProcessor);

impl DvcProcessor for EnumerationProcessor {
    fn channel_name(&self) -> &str {
        self.inner.channel_name()
    }

    fn max_message_size(&self) -> Option<usize> {
        self.inner.max_message_size()
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let events = {
            let mut shared = self.shared.lock().expect("camera-input state poisoned");
            if shared.connected
                && shared.active
                && shared.generation == self.generation
                && shared.enumeration_channel_id == Some(channel_id)
            {
                shared.enumeration_requested = false;
                return self.inner.start(channel_id);
            }
            shared.events.clone()
        };
        if let Some(events) = events {
            let _ = events.send(self.token.close(channel_id));
        }
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.inner.process(channel_id, payload)
    }

    fn close(&mut self, channel_id: u32) {
        self.token.invalidate();
        let mut shared = self.shared.lock().expect("camera-input state poisoned");
        if shared.generation == self.generation && shared.enumeration_channel_id == Some(channel_id) {
            shared.enumeration_channel_id = None;
            shared.enumeration_requested = false;
            shared.version = None;
            shared.devices.clear();
        }
        self.inner.close(channel_id);
    }
}

impl DvcServerProcessor for EnumerationProcessor {}

struct DeviceProcessor {
    inner: Arc<Mutex<DeviceServer>>,
    channel_name: String,
    shared: Arc<Mutex<Shared>>,
    token: ChannelGuard,
    generation: u64,
}

impl_as_any!(DeviceProcessor);

impl DvcProcessor for DeviceProcessor {
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(ironrdp_rdpecam::pdu::MAX_SAMPLE_SIZE + 64)
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        if !is_device_active(&self.shared, channel_id, self.generation) {
            if let Some(events) = self.shared.lock().expect("camera-input state poisoned").events.clone() {
                let _ = events.send(self.token.close(channel_id));
            }
            return Ok(Vec::new());
        }
        self.inner
            .lock()
            .expect("camera device state poisoned")
            .start(channel_id)
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let (messages, stopped) = {
            let mut inner = self.inner.lock().expect("camera device state poisoned");
            let messages = inner.process(channel_id, payload)?;
            (messages, inner.is_stopped())
        };
        if stopped
            && let Some(events) = self.shared.lock().expect("camera-input state poisoned").events.clone()
            && let Err(error) = events.send(self.token.close(channel_id))
        {
            tracing::debug!(%error, "RDP server stopped before a deactivated camera channel was closed");
        }
        Ok(messages)
    }

    fn close(&mut self, channel_id: u32) {
        self.token.invalidate();
        {
            let mut shared = self.shared.lock().expect("camera-input state poisoned");
            if shared.device_channel.as_ref().is_some_and(|(id, _)| *id == channel_id)
                && shared
                    .device_token
                    .as_ref()
                    .is_some_and(|token| token.is_same_channel(&self.token))
            {
                shared.device_channel = None;
                shared.device_token = None;
                shared.device_server = None;
                shared.device_requested = None;
            }
        }
        self.inner
            .lock()
            .expect("camera device state poisoned")
            .close(channel_id);
    }
}

impl DvcServerProcessor for DeviceProcessor {}

fn stop_device_event(
    server: &Arc<Mutex<DeviceServer>>,
    channel_id: u32,
    token: ChannelGuard,
    permits: Arc<Semaphore>,
) -> anyhow::Result<Option<(ServerEvent, oneshot::Receiver<()>)>> {
    let Ok(permit) = permits.try_acquire_owned() else {
        return Ok(None);
    };
    let Some(message) = server.lock().expect("camera device state poisoned").stop() else {
        return Ok(None);
    };
    let messages = encode_dvc_messages(channel_id, vec![message], ChannelFlags::SHOW_PROTOCOL)?;
    let batch = DvcMessageBatch::try_new(channel_id, messages)?;
    let (completion, receiver) = oneshot::channel();
    Ok(Some((
        ServerEvent::DynamicChannelWithCompletion {
            batch,
            completion,
            permit,
            is_current: Box::new(move || token.is_current()),
        },
        receiver,
    )))
}

fn pixel_format(format: MediaFormat) -> Option<PixelFormat> {
    match format {
        MediaFormat::I420 => Some(PixelFormat::I420),
        MediaFormat::Nv12 => Some(PixelFormat::Nv12),
        MediaFormat::Yuy2 => Some(PixelFormat::Yuy2),
        MediaFormat::Mjpg => Some(PixelFormat::Mjpeg),
        MediaFormat::H264 | MediaFormat::Rgb24 | MediaFormat::Rgb32 => None,
    }
}

fn decode_h264_to_i420(
    decoder: &mut openh264::decoder::Decoder,
    sample: &[u8],
    packed: &mut Vec<u8>,
) -> anyhow::Result<Option<(u32, u32)>> {
    use openh264::formats::YUVSource as _;

    validate_h264_sps_dimensions(sample)?;
    let Some(yuv) = decoder.decode(sample)? else {
        return Ok(None);
    };
    let (width, height) = yuv.dimensions();
    let (Ok(w32), Ok(h32)) = (u32::try_from(width), u32::try_from(height)) else {
        anyhow::bail!("decoded H.264 camera dimensions overflow u32");
    };
    anyhow::ensure!(
        0 < w32 && w32 <= MAX_CEF_CAMERA_DIMENSION && 0 < h32 && h32 <= MAX_CEF_CAMERA_DIMENSION,
        "decoded H.264 camera frame {w32}x{h32} exceeds bounds"
    );
    anyhow::ensure!(
        w32.is_multiple_of(2) && h32.is_multiple_of(2),
        "decoded H.264 camera frame {w32}x{h32} must have even dimensions"
    );
    let (y_stride, u_stride, v_stride) = yuv.strides();
    let uv_w = width / 2;
    let uv_h = height / 2;
    anyhow::ensure!(
        width <= y_stride && uv_w <= u_stride && uv_w <= v_stride,
        "decoded H.264 plane strides are smaller than frame dimensions"
    );
    let y_plane = yuv.y();
    let u_plane = yuv.u();
    let v_plane = yuv.v();
    anyhow::ensure!(
        height.saturating_mul(y_stride) <= y_plane.len()
            && uv_h.saturating_mul(u_stride) <= u_plane.len()
            && uv_h.saturating_mul(v_stride) <= v_plane.len(),
        "decoded H.264 planes are shorter than expected"
    );
    let total = width * height + 2 * uv_w * uv_h;
    packed.clear();
    packed.reserve(total);
    for row in 0..height {
        let start = row * y_stride;
        packed.extend_from_slice(&y_plane[start..start + width]);
    }
    for row in 0..uv_h {
        let start = row * u_stride;
        packed.extend_from_slice(&u_plane[start..start + uv_w]);
    }
    for row in 0..uv_h {
        let start = row * v_stride;
        packed.extend_from_slice(&v_plane[start..start + uv_w]);
    }
    Ok(Some((w32, h32)))
}

fn validate_h264_sps_dimensions(sample: &[u8]) -> anyhow::Result<()> {
    for nal in iter_annex_b_nalus(sample) {
        let Some(&header) = nal.first() else {
            continue;
        };
        if header & 0x80 != 0 {
            anyhow::bail!("H.264 NAL forbidden_zero_bit is set");
        }
        // Validate both seq_parameter_set_rbsp (7) and subset_seq_parameter_set_rbsp (15),
        // which begins with the identical seq_parameter_set_data() header.
        if !matches!(header & 0x1F, 7 | 15) {
            continue;
        }
        let rbsp = unescape_rbsp(&nal[1..], 4096);
        let mut bits = ExpGolombReader::new(&rbsp);
        let profile_idc = bits.read_bits(8)?;
        let _constraints = bits.read_bits(8)?;
        let _level_idc = bits.read_bits(8)?;
        let sps_id = bits.read_ue()?;
        anyhow::ensure!(sps_id <= 31, "H.264 seq_parameter_set_id out of range");
        if matches!(
            profile_idc,
            100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
        ) {
            let chroma_format_idc = bits.read_ue()?;
            anyhow::ensure!(chroma_format_idc <= 3, "invalid H.264 chroma_format_idc");
            if chroma_format_idc == 3 {
                let _separate_colour_plane = bits.read_bits(1)?;
            }
            let _bit_depth_luma = bits.read_ue()?;
            let _bit_depth_chroma = bits.read_ue()?;
            let _qpprime_bypass = bits.read_bits(1)?;
            if bits.read_bits(1)? == 1 {
                let lists = if chroma_format_idc != 3 { 8 } else { 12 };
                for index in 0..lists {
                    if bits.read_bits(1)? == 1 {
                        let size = if index < 6 { 16 } else { 64 };
                        let mut last_scale = 8i64;
                        let mut next_scale = 8i64;
                        for _ in 0..size {
                            if next_scale != 0 {
                                let delta = bits.read_se()?;
                                next_scale = (last_scale + delta + 256).rem_euclid(256);
                            }
                            if next_scale != 0 {
                                last_scale = next_scale;
                            }
                        }
                    }
                }
            }
        }
        let _log2_max_frame_num = bits.read_ue()?;
        let poc_type = bits.read_ue()?;
        match poc_type {
            0 => {
                let _log2_max_poc_lsb = bits.read_ue()?;
            }
            1 => {
                let _delta_always_zero = bits.read_bits(1)?;
                let _offset_non_ref = bits.read_se()?;
                let _offset_top_bottom = bits.read_se()?;
                let cycle = bits.read_ue()?;
                anyhow::ensure!(cycle <= 255, "H.264 POC cycle length out of range");
                for _ in 0..cycle {
                    let _ = bits.read_se()?;
                }
            }
            2 => {}
            _ => anyhow::bail!("invalid H.264 pic_order_cnt_type"),
        }
        let max_ref_frames = bits.read_ue()?;
        anyhow::ensure!(max_ref_frames <= 16, "H.264 max_num_ref_frames exceeds 16");
        let _gaps_allowed = bits.read_bits(1)?;
        let width_mbs = bits.read_ue()?.saturating_add(1);
        let height_map_units = bits.read_ue()?.saturating_add(1);
        let frame_mbs_only = bits.read_bits(1)?;
        let coded_width = width_mbs.saturating_mul(16);
        let coded_height = height_map_units
            .saturating_mul(16)
            .saturating_mul(2u32.saturating_sub(frame_mbs_only));
        anyhow::ensure!(
            coded_width <= MAX_CEF_CAMERA_DIMENSION && coded_height <= MAX_CEF_CAMERA_DIMENSION,
            "H.264 SPS coded dimensions {coded_width}x{coded_height} exceed {MAX_CEF_CAMERA_DIMENSION}"
        );
    }
    Ok(())
}

fn iter_annex_b_nalus(data: &[u8]) -> AnnexBNalIter<'_> {
    AnnexBNalIter {
        data,
        next_payload: find_start_code(data, 0).map(|(_, after)| after),
    }
}

struct AnnexBNalIter<'a> {
    data: &'a [u8],
    next_payload: Option<usize>,
}

impl<'a> Iterator for AnnexBNalIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.next_payload?;
        match find_start_code(self.data, start) {
            Some((code_start, next_after)) => {
                self.next_payload = Some(next_after);
                Some(&self.data[start..code_start])
            }
            None => {
                self.next_payload = None;
                Some(&self.data[start..])
            }
        }
    }
}

fn find_start_code(data: &[u8], mut idx: usize) -> Option<(usize, usize)> {
    while idx + 3 <= data.len() {
        if data[idx] == 0 && data[idx + 1] == 0 {
            if data[idx + 2] == 1 {
                return Some((idx, idx + 3));
            }
            if idx + 4 <= data.len() && data[idx + 2] == 0 && data[idx + 3] == 1 {
                return Some((idx, idx + 4));
            }
        }
        idx += 1;
    }
    None
}

fn unescape_rbsp(ebsp: &[u8], limit: usize) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(ebsp.len().min(limit));
    let mut zeros = 0usize;
    for &byte in ebsp {
        if rbsp.len() >= limit {
            break;
        }
        if zeros >= 2 && byte == 0x03 {
            zeros = 0;
            continue;
        }
        rbsp.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    rbsp
}

struct ExpGolombReader<'a> {
    bytes: &'a [u8],
    bit_offset: usize,
}

impl<'a> ExpGolombReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit_offset: 0 }
    }

    fn read_bit(&mut self) -> anyhow::Result<u32> {
        let byte_idx = self.bit_offset / 8;
        let Some(&byte) = self.bytes.get(byte_idx) else {
            anyhow::bail!("truncated H.264 SPS RBSP");
        };
        let shift = 7 - (self.bit_offset % 8);
        self.bit_offset += 1;
        Ok(u32::from((byte >> shift) & 1))
    }

    fn read_bits(&mut self, count: usize) -> anyhow::Result<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | self.read_bit()?;
        }
        Ok(value)
    }

    fn read_ue(&mut self) -> anyhow::Result<u32> {
        let mut leading_zeros = 0usize;
        while self.read_bit()? == 0 {
            leading_zeros += 1;
            anyhow::ensure!(leading_zeros <= 31, "Exp-Golomb code exceeds 32 bits");
        }
        if leading_zeros == 0 {
            return Ok(0);
        }
        let suffix = self.read_bits(leading_zeros)?;
        Ok(((1u32 << leading_zeros) - 1).saturating_add(suffix))
    }

    fn read_se(&mut self) -> anyhow::Result<i64> {
        let code = i64::from(self.read_ue()?);
        let magnitude = (code + 1) / 2;
        Ok(if code % 2 == 1 { magnitude } else { -magnitude })
    }
}

fn is_device_active(shared: &Arc<Mutex<Shared>>, channel_id: u32, generation: u64) -> bool {
    let shared = shared.lock().expect("camera-input state poisoned");
    shared.connected
        && shared.active
        && shared.generation == generation
        && shared.device_channel.as_ref().is_some_and(|(id, _)| *id == channel_id)
}

struct UnstartedCameraChannel {
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    request: Option<Arc<str>>,
    armed: bool,
}

impl UnstartedCameraChannel {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UnstartedCameraChannel {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = self.shared.lock().expect("camera-input state poisoned");
        if !(state.connected && state.active && state.generation == self.generation) {
            return;
        }
        match &self.request {
            None if state.enumeration_channel_id.is_none() => {
                state.enumeration_requested = false;
            }
            Some(request)
                if state.device_channel.is_none()
                    && state
                        .device_requested
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, request)) =>
            {
                state.device_requested = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{c_float, c_int};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DvcServerProcessor;
    use ironrdp_rdpecam::pdu::{DeviceMessage, DevicePdu, EnumerationPdu, ErrorCode, StreamDescription};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use super::*;

    static CAPTURING: AtomicBool = AtomicBool::new(true);
    static PUSHED_FRAMES: AtomicUsize = AtomicUsize::new(0);
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    unsafe extern "C" fn get_format(width: *mut c_int, height: *mut c_int, fps: *mut c_float) -> c_int {
        if !CAPTURING.load(Ordering::Relaxed) {
            return 0;
        }
        // SAFETY: RemoteMedia supplies valid writable pointers for these three outputs.
        unsafe {
            *width = 640;
            *height = 480;
            *fps = 30.0;
        }
        1
    }

    unsafe extern "C" fn push_frame(_: c_int, _: c_int, _: c_int, _: *const u8, _: usize, _: i64) -> c_int {
        PUSHED_FRAMES.fetch_add(1, Ordering::Relaxed);
        0
    }

    unsafe extern "C" fn microphone_active() -> c_int {
        0
    }

    unsafe extern "C" fn push_audio(_: *const i16, _: usize, _: c_int, _: c_int) -> c_int {
        0
    }

    fn create_channel(events: &mut UnboundedReceiver<ServerEvent>, channel_id: u32) -> Box<dyn DvcServerProcessor> {
        let ServerEvent::CreateDynamicChannel(build) = events.try_recv().expect("expected channel creation") else {
            panic!("expected channel creation");
        };
        let mut channel = build(channel_id);
        channel.start(channel_id).unwrap();
        channel
    }

    fn decode_message(messages: &[DvcMessage]) -> DeviceMessage {
        let encoded = encode_vec(messages.first().expect("expected a device message").as_ref()).unwrap();
        ironrdp_core::decode(&encoded).unwrap()
    }

    fn announce_camera(events: &mut UnboundedReceiver<ServerEvent>, channel_id: u32) {
        let mut enumeration = create_channel(events, channel_id);
        enumeration
            .process(
                channel_id,
                &encode_vec(&EnumerationPdu::SelectVersionRequest(ProtocolVersion::V2)).unwrap(),
            )
            .unwrap();
        let device = DeviceDescriptor::new("Camera".into(), "RDCamera_Device_0".into()).unwrap();
        enumeration
            .process(
                channel_id,
                &encode_vec(&EnumerationPdu::DeviceAdded {
                    version: ProtocolVersion::V2,
                    device,
                })
                .unwrap(),
            )
            .unwrap();
    }

    fn camera_channel() -> (CameraInput, UnboundedReceiver<ServerEvent>, Box<dyn DvcServerProcessor>) {
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(get_format, push_frame, microphone_active, push_audio);
        let mut camera = CameraInput::new(media);
        let (sender, mut events) = unbounded_channel();
        camera.set_sender(sender);
        camera.connected();
        camera.tick();
        announce_camera(&mut events, 1);
        camera.tick();
        let ServerEvent::CreateDynamicChannel(build) = events.try_recv().unwrap() else {
            panic!("expected device channel");
        };
        let channel = build(2);
        (camera, events, channel)
    }

    #[test]
    fn rejected_camera_setup_does_not_retry_until_capture_restarts() {
        // oracle: mutant repeatedly reopening a failed camera while the same capture request remains active.
        let _guard = TEST_LOCK.lock().unwrap();
        let (camera, mut events, mut channel) = camera_channel();
        channel.start(2).unwrap();
        channel
            .process(
                2,
                &encode_vec(&DeviceMessage::new(
                    ProtocolVersion::V2,
                    DevicePdu::ErrorResponse(ErrorCode::Unexpected),
                ))
                .unwrap(),
            )
            .unwrap();
        let ServerEvent::CloseDynamicChannel { channel_id, is_current } = events.try_recv().unwrap() else {
            panic!("expected channel close");
        };
        assert_eq!(channel_id, 2);
        assert!(is_current());
        channel.close(channel_id);

        camera.tick();
        assert!(events.try_recv().is_err(), "failed camera was reopened");

        CAPTURING.store(false, Ordering::Relaxed);
        camera.tick();
        let ServerEvent::CloseDynamicChannel { channel_id, is_current } = events.try_recv().unwrap() else {
            panic!("expected enumeration channel close");
        };
        assert_eq!(channel_id, 1);
        assert!(is_current());
        CAPTURING.store(true, Ordering::Relaxed);
        camera.tick();
        assert!(matches!(events.try_recv(), Ok(ServerEvent::CreateDynamicChannel(_))));
    }

    #[test]
    fn declined_enumeration_is_retried_after_capture_restarts() {
        // oracle: mutant retaining a declined enumeration channel after capture ends.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(get_format, push_frame, microphone_active, push_audio);
        let mut camera = CameraInput::new(media);
        let (sender, mut events) = unbounded_channel();
        camera.set_sender(sender);
        camera.connected();
        camera.tick();
        let ServerEvent::CreateDynamicChannel(build) = events.try_recv().unwrap() else {
            panic!("expected channel creation");
        };
        let _declined = build(1);

        CAPTURING.store(false, Ordering::Relaxed);
        camera.tick();
        let ServerEvent::CloseDynamicChannel { channel_id, is_current } = events.try_recv().unwrap() else {
            panic!("expected cancellation of declined enumeration");
        };
        assert_eq!(channel_id, 1);
        assert!(is_current());
        CAPTURING.store(true, Ordering::Relaxed);
        camera.tick();
        assert!(matches!(events.try_recv(), Ok(ServerEvent::CreateDynamicChannel(_))));
    }

    #[test]
    fn late_enumeration_acceptance_does_not_replace_the_new_capture_channel() {
        // oracle: mutant letting a late channel acceptance overwrite the current capture's channel.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(get_format, push_frame, microphone_active, push_audio);
        let mut camera = CameraInput::new(media);
        let (sender, mut events) = unbounded_channel();
        camera.set_sender(sender);
        camera.connected();
        camera.tick();
        let ServerEvent::CreateDynamicChannel(old) = events.try_recv().unwrap() else {
            panic!("expected channel creation");
        };
        CAPTURING.store(false, Ordering::Relaxed);
        camera.tick();
        CAPTURING.store(true, Ordering::Relaxed);
        camera.tick();
        let ServerEvent::CreateDynamicChannel(new) = events.try_recv().unwrap() else {
            panic!("expected replacement channel");
        };
        let mut old_channel = old(1);
        let _new_channel = new(2);
        old_channel.start(1).unwrap();
        CAPTURING.store(false, Ordering::Relaxed);
        camera.tick();
        let mut closes: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|event| match event {
                ServerEvent::CloseDynamicChannel { channel_id, is_current } => Some((channel_id, is_current())),
                _ => None,
            })
            .collect();
        closes.sort_unstable();
        closes.dedup();
        assert_eq!(closes, [(1, true), (2, true)]);
    }

    #[test]
    fn old_enumeration_close_cannot_clear_replacement_after_channel_id_reuse() {
        // oracle: mutant an old enumeration close clears a new capture's channel.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(get_format, push_frame, microphone_active, push_audio);
        let mut camera = CameraInput::new(media);
        let (sender, mut events) = unbounded_channel();
        camera.set_sender(sender);
        camera.connected();
        camera.tick();
        let mut old_channel = create_channel(&mut events, 1);

        CAPTURING.store(false, Ordering::Relaxed);
        camera.tick();
        let ServerEvent::CloseDynamicChannel { channel_id, .. } = events.try_recv().unwrap() else {
            panic!("expected old enumeration close");
        };
        assert_eq!(channel_id, 1);
        CAPTURING.store(true, Ordering::Relaxed);
        camera.tick();
        let mut replacement = create_channel(&mut events, 1);

        old_channel.close(1);
        camera.tick();
        assert!(events.try_recv().is_err(), "old close triggered a new enumeration");

        replacement
            .process(
                1,
                &encode_vec(&EnumerationPdu::SelectVersionRequest(ProtocolVersion::V2)).unwrap(),
            )
            .unwrap();
        let device = DeviceDescriptor::new("Camera".into(), "RDCamera_Device_0".into()).unwrap();
        replacement
            .process(
                1,
                &encode_vec(&EnumerationPdu::DeviceAdded {
                    version: ProtocolVersion::V2,
                    device: device.clone(),
                })
                .unwrap(),
            )
            .unwrap();
        camera.tick();
        let channel = create_channel(&mut events, 2);
        assert_eq!(channel.channel_name(), device.channel_name);
    }

    #[test]
    fn old_camera_channel_cannot_deliver_frames_after_capture_restarts() {
        // oracle: mutant delivering a frame from an earlier capture to a later capture.
        let _guard = TEST_LOCK.lock().unwrap();
        let (camera, _events, mut channel) = camera_channel();
        channel.start(2).unwrap();
        let message = |pdu| encode_vec(&DeviceMessage::new(ProtocolVersion::V2, pdu)).unwrap();
        channel.process(2, &message(DevicePdu::SuccessResponse)).unwrap();
        channel
            .process(
                2,
                &message(DevicePdu::StreamListResponse(vec![StreamDescription::color(
                    true, false,
                )])),
            )
            .unwrap();
        let media_type = MediaType::new(MediaFormat::I420, 4, 4, 30, 1, 1, 1, 0).unwrap();
        channel
            .process(2, &message(DevicePdu::MediaTypeListResponse(vec![media_type])))
            .unwrap();
        channel.process(2, &message(DevicePdu::SuccessResponse)).unwrap();
        let sample = message(DevicePdu::SampleResponse {
            stream_index: 0,
            sample: vec![0; 24],
        });
        PUSHED_FRAMES.store(0, Ordering::Relaxed);
        channel.process(2, &sample).unwrap();
        assert_eq!(PUSHED_FRAMES.load(Ordering::Relaxed), 1);

        CAPTURING.store(false, Ordering::Relaxed);
        camera.tick();
        CAPTURING.store(true, Ordering::Relaxed);
        camera.tick();
        channel.process(2, &sample).unwrap();
        assert_eq!(PUSHED_FRAMES.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn capture_stop_sends_protocol_shutdown_before_closing_device_channel() {
        // oracle: spec MS-RDPECAM §1.3.4 requires Stop Streams before Deactivate Device.
        let _guard = TEST_LOCK.lock().unwrap();
        let (camera, mut events, mut channel) = camera_channel();
        channel.start(2).unwrap();
        let message = |pdu| encode_vec(&DeviceMessage::new(ProtocolVersion::V2, pdu)).unwrap();
        channel.process(2, &message(DevicePdu::SuccessResponse)).unwrap();
        channel
            .process(
                2,
                &message(DevicePdu::StreamListResponse(vec![StreamDescription::color(
                    true, false,
                )])),
            )
            .unwrap();
        let media_type = MediaType::new(MediaFormat::I420, 4, 4, 30, 1, 1, 1, 0).unwrap();
        channel
            .process(2, &message(DevicePdu::MediaTypeListResponse(vec![media_type])))
            .unwrap();
        channel.process(2, &message(DevicePdu::SuccessResponse)).unwrap();

        CAPTURING.store(false, Ordering::Relaxed);
        camera.tick();

        let (batch, completion, is_current) = std::iter::from_fn(|| events.try_recv().ok())
            .find_map(|event| match event {
                ServerEvent::DynamicChannelWithCompletion {
                    batch,
                    completion,
                    is_current,
                    ..
                } => Some((batch, completion, is_current)),
                _ => None,
            })
            .expect("capture stop must send a protocol shutdown message before closing the channel");
        assert_eq!(batch.channel_id(), 2);
        assert_eq!(batch.messages().len(), 1);
        assert!(is_current());
        let encoded = batch.messages()[0].encode_unframed_pdu().unwrap();
        let dynamic_pdu: ironrdp_dvc::pdu::DrdynvcClientPdu = ironrdp_core::decode(&encoded).unwrap();
        let ironrdp_dvc::pdu::DrdynvcClientPdu::Data(ironrdp_dvc::pdu::DrdynvcDataPdu::Data(data)) = dynamic_pdu else {
            panic!("camera shutdown must use a complete dynamic-channel data PDU");
        };
        let shutdown: DeviceMessage = ironrdp_core::decode(data.data()).unwrap();
        assert_eq!(shutdown.pdu, DevicePdu::StopStreamsRequest);
        completion.send(()).unwrap();
        camera.tick();

        let success = encode_vec(&DeviceMessage::new(ProtocolVersion::V2, DevicePdu::SuccessResponse)).unwrap();
        let deactivate = channel.process(2, &success).unwrap();
        assert_eq!(
            decode_message(&deactivate),
            DeviceMessage::new(ProtocolVersion::V2, DevicePdu::DeactivateDeviceRequest)
        );
        assert!(channel.process(2, &success).unwrap().is_empty());
        let ServerEvent::CloseDynamicChannel { channel_id, is_current } = events.try_recv().unwrap() else {
            panic!("deactivated camera channel must close");
        };
        assert_eq!(channel_id, 2);
        assert!(is_current());
    }

    #[test]
    fn late_device_acceptance_cannot_close_a_channel_on_the_next_connection() {
        // oracle: mutant reviving a stale close guard after the previous RDP connection ends.
        let _guard = TEST_LOCK.lock().unwrap();
        let (camera, mut events, mut channel) = camera_channel();
        camera.disconnected();
        camera.connected();
        channel.start(2).unwrap();
        let ServerEvent::CloseDynamicChannel { is_current, .. } = events.try_recv().unwrap() else {
            panic!("expected stale channel close");
        };
        assert!(!is_current(), "a close from the old connection is still valid");
    }

    #[test]
    fn queued_camera_close_is_invalid_after_reconnect() {
        // oracle: mutant applying a queued close to a reused channel ID on another connection.
        let _guard = TEST_LOCK.lock().unwrap();
        let (camera, mut events, mut channel) = camera_channel();
        channel.start(2).unwrap();
        CAPTURING.store(false, Ordering::Relaxed);
        camera.tick();
        camera.disconnected();
        camera.connected();
        let ServerEvent::CloseDynamicChannel { is_current, .. } = events.try_recv().unwrap() else {
            panic!("expected channel close");
        };
        assert!(!is_current(), "the old connection's close remained valid");
    }

    #[test]
    fn stale_camera_factory_does_not_duplicate_a_new_request() {
        // oracle: mutant clearing a newer pending request when an old factory runs.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(get_format, push_frame, microphone_active, push_audio);
        let mut camera = CameraInput::new(media);
        let (sender, mut events) = unbounded_channel();
        camera.set_sender(sender);
        camera.connected();
        camera.tick();
        announce_camera(&mut events, 1);
        camera.tick();
        let ServerEvent::CreateDynamicChannel(old) = events.try_recv().unwrap() else {
            panic!("expected device channel");
        };
        CAPTURING.store(false, Ordering::Relaxed);
        camera.tick();
        assert!(matches!(events.try_recv(), Ok(ServerEvent::CloseDynamicChannel { .. })));
        CAPTURING.store(true, Ordering::Relaxed);
        camera.tick();
        announce_camera(&mut events, 2);
        camera.tick();
        let ServerEvent::CreateDynamicChannel(_new) = events.try_recv().unwrap() else {
            panic!("expected replacement device channel");
        };
        let _stale = old(3);
        assert!(matches!(events.try_recv(), Ok(ServerEvent::CloseDynamicChannel { .. })));
        camera.tick();
        assert!(events.try_recv().is_err(), "a duplicate creation was queued");
    }

    #[test]
    fn removed_camera_cannot_start_from_a_queued_channel_creation() {
        // oracle: mutant starting an old camera request after the device is removed and re-added.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(get_format, push_frame, microphone_active, push_audio);
        let mut camera = CameraInput::new(media);
        let (sender, mut events) = unbounded_channel();
        camera.set_sender(sender);
        camera.connected();
        camera.tick();
        let mut enumeration = create_channel(&mut events, 1);
        let message = |pdu| encode_vec(&pdu).unwrap();
        enumeration
            .process(1, &message(EnumerationPdu::SelectVersionRequest(ProtocolVersion::V2)))
            .unwrap();
        let device = DeviceDescriptor::new("Camera".into(), "RDCamera_Device_0".into()).unwrap();
        let added = || EnumerationPdu::DeviceAdded {
            version: ProtocolVersion::V2,
            device: device.clone(),
        };
        enumeration.process(1, &message(added())).unwrap();
        camera.tick();
        let ServerEvent::CreateDynamicChannel(old) = events.try_recv().unwrap() else {
            panic!("expected camera channel creation");
        };
        enumeration
            .process(
                1,
                &message(EnumerationPdu::DeviceRemoved {
                    version: ProtocolVersion::V2,
                    channel_name: device.channel_name.clone(),
                }),
            )
            .unwrap();
        enumeration.process(1, &message(added())).unwrap();
        camera.tick();
        let ServerEvent::CreateDynamicChannel(replacement) = events.try_recv().unwrap() else {
            panic!("expected replacement camera channel creation");
        };
        let mut stale = old(2);
        assert!(stale.start(2).unwrap().is_empty(), "removed camera was activated");
        let mut current = replacement(3);
        assert_eq!(current.start(3).unwrap().len(), 1);
    }

    #[test]
    fn removed_camera_cannot_deliver_frames_after_readdition() {
        // oracle: mutant delivering frames from a removed camera after its name is re-advertised.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(get_format, push_frame, microphone_active, push_audio);
        let mut camera = CameraInput::new(media);
        let (sender, mut events) = unbounded_channel();
        camera.set_sender(sender);
        camera.connected();
        camera.tick();
        let mut enumeration = create_channel(&mut events, 1);
        let enumeration_message = |pdu| encode_vec(&pdu).unwrap();
        enumeration
            .process(
                1,
                &enumeration_message(EnumerationPdu::SelectVersionRequest(ProtocolVersion::V2)),
            )
            .unwrap();
        let device = DeviceDescriptor::new("Camera".into(), "RDCamera_Device_0".into()).unwrap();
        let added = || EnumerationPdu::DeviceAdded {
            version: ProtocolVersion::V2,
            device: device.clone(),
        };
        enumeration.process(1, &enumeration_message(added())).unwrap();
        camera.tick();
        let mut channel = create_channel(&mut events, 2);
        let message = |pdu| encode_vec(&DeviceMessage::new(ProtocolVersion::V2, pdu)).unwrap();
        channel.process(2, &message(DevicePdu::SuccessResponse)).unwrap();
        channel
            .process(
                2,
                &message(DevicePdu::StreamListResponse(vec![StreamDescription::color(
                    true, false,
                )])),
            )
            .unwrap();
        let media_type = MediaType::new(MediaFormat::I420, 4, 4, 30, 1, 1, 1, 0).unwrap();
        channel
            .process(2, &message(DevicePdu::MediaTypeListResponse(vec![media_type])))
            .unwrap();
        channel.process(2, &message(DevicePdu::SuccessResponse)).unwrap();
        let sample = message(DevicePdu::SampleResponse {
            stream_index: 0,
            sample: vec![0; 24],
        });
        PUSHED_FRAMES.store(0, Ordering::Relaxed);
        channel.process(2, &sample).unwrap();
        assert_eq!(PUSHED_FRAMES.load(Ordering::Relaxed), 1);
        enumeration
            .process(
                1,
                &enumeration_message(EnumerationPdu::DeviceRemoved {
                    version: ProtocolVersion::V2,
                    channel_name: device.channel_name.clone(),
                }),
            )
            .unwrap();
        enumeration.process(1, &enumeration_message(added())).unwrap();
        camera.tick();
        channel.process(2, &sample).unwrap();
        assert_eq!(PUSHED_FRAMES.load(Ordering::Relaxed), 1);
        let ServerEvent::CloseDynamicChannel { channel_id, is_current } = events.try_recv().unwrap() else {
            panic!("expected removed camera channel close");
        };
        assert_eq!(channel_id, 2);
        assert!(is_current());
        let mut replacement = create_channel(&mut events, 3);
        channel.close(2);
        replacement.process(3, &message(DevicePdu::SuccessResponse)).unwrap();
        replacement
            .process(
                3,
                &message(DevicePdu::StreamListResponse(vec![StreamDescription::color(
                    true, false,
                )])),
            )
            .unwrap();
        replacement
            .process(3, &message(DevicePdu::MediaTypeListResponse(vec![media_type])))
            .unwrap();
        replacement.process(3, &message(DevicePdu::SuccessResponse)).unwrap();
        replacement.process(3, &sample).unwrap();
        assert_eq!(PUSHED_FRAMES.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn unready_drdynvc_drop_retries_enumeration_request() {
        // oracle: mutant permanently wedging camera enumeration when DRDYNVC drops an early CreateDynamicChannel.
        let _guard = TEST_LOCK.lock().unwrap();
        CAPTURING.store(true, Ordering::Relaxed);
        let media = RemoteMedia::for_test(get_format, push_frame, microphone_active, push_audio);
        let mut camera = CameraInput::new(media);
        let (sender, mut events) = unbounded_channel();
        camera.set_sender(sender);
        camera.connected();
        camera.tick();
        let unready = events.try_recv().expect("initial enumeration request");
        drop(unready);

        camera.tick();
        assert!(matches!(events.try_recv(), Ok(ServerEvent::CreateDynamicChannel(_))));
    }

    #[test]
    fn h264_samples_are_decoded_to_packed_i420_and_pushed_to_cef() {
        // oracle: mutant rejecting MediaFormat::H264 or failing to unpack strided OpenH264 planes into tight I420.
        let _guard = TEST_LOCK.lock().unwrap();
        let (_camera, _events, mut channel) = camera_channel();
        channel.start(2).unwrap();
        let message = |pdu| encode_vec(&DeviceMessage::new(ProtocolVersion::V2, pdu)).unwrap();
        channel.process(2, &message(DevicePdu::SuccessResponse)).unwrap();
        channel
            .process(
                2,
                &message(DevicePdu::StreamListResponse(vec![StreamDescription::color(
                    true, false,
                )])),
            )
            .unwrap();
        let media_type = MediaType::new(MediaFormat::H264, 16, 16, 30, 1, 1, 1, MediaType::DECODING_REQUIRED).unwrap();
        let start = decode_message(
            &channel
                .process(2, &message(DevicePdu::MediaTypeListResponse(vec![media_type])))
                .unwrap(),
        );
        assert_eq!(
            start.pdu,
            DevicePdu::StartStreamsRequest(vec![ironrdp_rdpecam::pdu::StartStreamInfo {
                stream_index: 0,
                media_type,
            }])
        );
        channel.process(2, &message(DevicePdu::SuccessResponse)).unwrap();

        let mut encoder = openh264::encoder::Encoder::new().unwrap();
        let mut i420 = vec![149u8; 16 * 16];
        i420.extend(std::iter::repeat_n(43u8, 8 * 8));
        i420.extend(std::iter::repeat_n(21u8, 8 * 8));
        let yuv =
            openh264::formats::YUVSlices::new((&i420[..256], &i420[256..320], &i420[320..384]), (16, 16), (16, 8, 8));
        let annex_b = encoder.encode(&yuv).unwrap().to_vec();
        assert!(media_type.validate_sample(&annex_b));

        PUSHED_FRAMES.store(0, Ordering::Relaxed);
        let next = decode_message(
            &channel
                .process(
                    2,
                    &message(DevicePdu::SampleResponse {
                        stream_index: 0,
                        sample: annex_b,
                    }),
                )
                .unwrap(),
        );
        assert_eq!(next.pdu, DevicePdu::SampleRequest(0));
        assert_eq!(PUSHED_FRAMES.load(Ordering::Relaxed), 1);

        let at_limit_sps = encode_baseline_sps(255, 255);
        assert!(validate_h264_sps_dimensions(&at_limit_sps).is_ok());

        // Coded dimensions 4112x4096 (257x256 macroblocks) exceed MAX_CEF_CAMERA_DIMENSION (4096)
        // and must be rejected before OpenH264 allocates reference buffers.
        let oversized_sps = encode_baseline_sps(256, 255);
        assert!(validate_h264_sps_dimensions(&oversized_sps).is_err());
        let mut oversized_subset_sps = oversized_sps.clone();
        oversized_subset_sps[4] = 0x6F;
        assert!(validate_h264_sps_dimensions(&oversized_subset_sps).is_err());
        channel
            .process(
                2,
                &message(DevicePdu::SampleResponse {
                    stream_index: 0,
                    sample: oversized_sps,
                }),
            )
            .unwrap();
        assert_eq!(PUSHED_FRAMES.load(Ordering::Relaxed), 1);
    }

    fn encode_baseline_sps(width_mbs_minus1: u32, height_mbs_minus1: u32) -> Vec<u8> {
        fn push_bit(bits: &mut Vec<u8>, bit: u8) {
            bits.push(bit & 1);
        }
        fn push_ue(bits: &mut Vec<u8>, value: u32) {
            let code = value + 1;
            let zeros = 31 - code.leading_zeros();
            for _ in 0..zeros {
                push_bit(bits, 0);
            }
            for shift in (0..=zeros).rev() {
                push_bit(bits, u8::try_from((code >> shift) & 1).expect("single bit"));
            }
        }
        let mut out = vec![0, 0, 0, 1, 0x67, 66, 0x00, 51];
        let mut bits = Vec::new();
        push_ue(&mut bits, 0);
        push_ue(&mut bits, 0);
        push_ue(&mut bits, 2);
        push_ue(&mut bits, 1);
        push_bit(&mut bits, 0);
        push_ue(&mut bits, width_mbs_minus1);
        push_ue(&mut bits, height_mbs_minus1);
        push_bit(&mut bits, 1);
        push_bit(&mut bits, 1);
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for (idx, &bit) in chunk.iter().enumerate() {
                byte |= bit << (7 - idx);
            }
            if out.len() >= 6 && out[out.len() - 2] == 0 && out[out.len() - 1] == 0 && byte <= 3 {
                out.push(0x03);
            }
            out.push(byte);
        }
        out
    }
}
