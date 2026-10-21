//! Client state machines for the MS-RDPECAM dynamic virtual channels.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use ironrdp_core::{AsAny, decode, impl_as_any, invalid_field_err};
use ironrdp_dvc::{DvcChannelListener, DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, trace, warn};

use crate::ENUMERATION_CHANNEL_NAME;
use crate::pdu::{
    DeviceDescriptor, DeviceMessage, DevicePdu, EnumerationPdu, ErrorCode, MAX_PROPERTIES, MediaType,
    PropertyDescription, PropertyId, PropertySet, PropertyValue, ProtocolVersion, StartStreamInfo, StreamDescription,
};

/// Largest number of unanswered Sample Requests accepted per stream.
pub const MAX_PENDING_SAMPLE_REQUESTS: u32 = 64;

/// Callback used to push already-wrapped DRDYNVC messages into the session loop.
///
/// Captured samples are produced outside the DVC processing call, so their Sample Responses are
/// sent through this callback instead of being returned from [`DvcProcessor::process`].
pub type SampleUplink = Arc<dyn Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnumerationState {
    WaitingVersion,
    Ready,
}

/// State shared by an enumeration channel and the devices it advertises.
#[derive(Debug)]
struct SessionShared {
    /// Negotiated protocol version, or zero before negotiation.
    version: AtomicU8,
}

impl SessionShared {
    fn version(&self) -> ProtocolVersion {
        ProtocolVersion::try_from(self.version.load(Ordering::Acquire)).unwrap_or(ProtocolVersion::V1)
    }
}

/// Shared redirection gate for an enumeration channel and its selected devices.
#[derive(Debug, Clone)]
pub struct CameraRedirectionSession {
    shared: Arc<SessionShared>,
}

impl CameraRedirectionSession {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(SessionShared {
                version: AtomicU8::new(0),
            }),
        }
    }

    /// Creates one camera selection that may be advertised by this session.
    pub fn select_device(&self, descriptor: DeviceDescriptor) -> ironrdp_core::EncodeResult<RedirectedDevice> {
        descriptor.validate()?;
        Ok(RedirectedDevice {
            descriptor,
            session: Arc::clone(&self.shared),
            advertised: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn enumeration_client(&self, devices: Vec<RedirectedDevice>) -> ironrdp_core::EncodeResult<EnumerationClient> {
        EnumerationClient::new(Arc::clone(&self.shared), devices)
    }

    /// Protocol version selected by the server, once negotiated.
    pub fn negotiated_version(&self) -> Option<ProtocolVersion> {
        ProtocolVersion::try_from(self.shared.version.load(Ordering::Acquire)).ok()
    }
}

impl Default for CameraRedirectionSession {
    fn default() -> Self {
        Self::new()
    }
}

/// One explicitly selected device shared by enumeration and device-channel processors.
#[derive(Debug, Clone)]
pub struct RedirectedDevice {
    descriptor: DeviceDescriptor,
    session: Arc<SessionShared>,
    advertised: Arc<AtomicBool>,
}

impl RedirectedDevice {
    pub fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    /// Creates the device-channel listener.
    ///
    /// `factory` is called each time the server opens the device channel; returning `None`
    /// declines the channel.
    pub fn listener<B, F>(&self, factory: F, uplink: SampleUplink) -> DeviceChannelListener<B>
    where
        B: CameraBackend,
        F: FnMut() -> Option<B> + Send + 'static,
    {
        DeviceChannelListener {
            device: self.clone(),
            state: Arc::new(spin::Mutex::new(DeviceState {
                backend: None,
                activation_depth: 0,
                holders: Vec::new(),
                sink: None,
                open_channels: 0,
            })),
            factory: Box::new(factory),
            uplink,
        }
    }
}

/// Client processor for the RDPECAM device-enumeration channel.
///
/// Only descriptors explicitly supplied by the caller are advertised.
#[derive(Debug)]
pub struct EnumerationClient {
    session: Arc<SessionShared>,
    devices: Vec<RedirectedDevice>,
    state: EnumerationState,
    channel_id: Option<u32>,
}

impl EnumerationClient {
    fn new(session: Arc<SessionShared>, devices: Vec<RedirectedDevice>) -> ironrdp_core::EncodeResult<Self> {
        if devices.len() > usize::from(u8::MAX) {
            return Err(invalid_field_err!(
                "devices",
                "at most 255 redirected cameras may be configured"
            ));
        }
        for (index, device) in devices.iter().enumerate() {
            device.descriptor.validate()?;
            if !Arc::ptr_eq(&session, &device.session) {
                return Err(invalid_field_err!(
                    "devices",
                    "redirected cameras must belong to the same redirection session"
                ));
            }
            if devices[..index]
                .iter()
                .any(|existing| existing.descriptor.channel_name == device.descriptor.channel_name)
            {
                return Err(invalid_field_err!(
                    "VirtualChannelName",
                    "redirected camera channel names must be unique"
                ));
            }
        }
        Ok(Self {
            session,
            devices,
            state: EnumerationState::WaitingVersion,
            channel_id: None,
        })
    }

    pub fn ready(&self) -> bool {
        self.state == EnumerationState::Ready
    }

    /// Adds a newly selected camera.
    ///
    /// Once the version is negotiated, the camera is advertised right away and the returned
    /// messages must be sent. Before that, it is advertised with the other cameras at negotiation
    /// and no message is returned.
    pub fn add_device(&mut self, device: RedirectedDevice) -> PduResult<Vec<SvcMessage>> {
        if !Arc::ptr_eq(&self.session, &device.session) {
            return Err(pdu_other_err!(
                "EnumerationClient::add_device",
                "redirected camera belongs to another redirection session"
            ));
        }
        if self.devices.len() == usize::from(u8::MAX) {
            return Err(pdu_other_err!(
                "EnumerationClient::add_device",
                "redirected camera limit has been reached"
            ));
        }
        if self
            .devices
            .iter()
            .any(|existing| existing.descriptor.channel_name == device.descriptor.channel_name)
        {
            return Err(pdu_other_err!(
                "EnumerationClient::add_device",
                "redirected camera channel name is already advertised"
            ));
        }
        if !self.ready() {
            self.devices.push(device);
            return Ok(Vec::new());
        }
        let channel_id = self.active_channel_id("EnumerationClient::add_device")?;
        let pdu = EnumerationPdu::DeviceAdded {
            version: self.session.version(),
            device: device.descriptor.clone(),
        };
        let messages = encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))?;
        device.advertised.store(true, Ordering::Release);
        self.devices.push(device);
        Ok(messages)
    }

    /// Removes a camera without activating or touching another device.
    ///
    /// Once the version is negotiated, the returned messages must be sent.
    pub fn remove_device(&mut self, channel_name: &str) -> PduResult<Vec<SvcMessage>> {
        let index = self
            .devices
            .iter()
            .position(|device| device.descriptor.channel_name == channel_name)
            .ok_or_else(|| {
                pdu_other_err!(
                    "EnumerationClient::remove_device",
                    "redirected camera channel name is not advertised"
                )
            })?;
        self.devices[index].advertised.store(false, Ordering::Release);
        if !self.ready() {
            self.devices.remove(index);
            return Ok(Vec::new());
        }
        let channel_id = self.active_channel_id("EnumerationClient::remove_device")?;
        let pdu = EnumerationPdu::DeviceRemoved {
            version: self.session.version(),
            channel_name: String::from(channel_name),
        };
        let messages = encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))?;
        self.devices.remove(index);
        Ok(messages)
    }

    /// Channel names of the currently advertised cameras.
    pub fn advertised_channels(&self) -> impl Iterator<Item = &str> {
        self.devices
            .iter()
            .map(|device| device.descriptor.channel_name.as_str())
    }

    fn active_channel_id(&self, context: &'static str) -> PduResult<u32> {
        if !self.ready() {
            return Err(pdu_other_err!(context, "camera enumeration channel is not ready"));
        }
        self.channel_id
            .ok_or_else(|| pdu_other_err!(context, "camera enumeration channel has no assigned channel id"))
    }
}

impl_as_any!(EnumerationClient);

impl DvcProcessor for EnumerationClient {
    fn channel_name(&self) -> &str {
        ENUMERATION_CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        for device in &self.devices {
            device.advertised.store(false, Ordering::Release);
        }
        self.session.version.store(0, Ordering::Release);
        self.channel_id = Some(channel_id);
        self.state = EnumerationState::WaitingVersion;
        debug!(channel_id, "Camera enumeration channel started");
        Ok(vec![Box::new(EnumerationPdu::SelectVersionRequest(
            ProtocolVersion::MAX,
        ))])
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if self.channel_id != Some(channel_id) {
            return Err(pdu_other_err!(
                "EnumerationClient::process",
                "camera enumeration channel id does not match the active channel"
            ));
        }
        let pdu: EnumerationPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring malformed camera enumeration PDU");
                return Ok(Vec::new());
            }
        };
        match pdu {
            // The decoder only produces versions this crate implements, all of which are at most
            // the version requested in Select Version Request.
            EnumerationPdu::SelectVersionResponse(version) if self.state == EnumerationState::WaitingVersion => {
                self.session.version.store(version.into(), Ordering::Release);
                for device in &self.devices {
                    device.advertised.store(true, Ordering::Release);
                }
                self.state = EnumerationState::Ready;
                debug!(?version, "Camera protocol version negotiated");
                Ok(self
                    .devices
                    .iter()
                    .map(|device| -> DvcMessage {
                        Box::new(EnumerationPdu::DeviceAdded {
                            version,
                            device: device.descriptor.clone(),
                        })
                    })
                    .collect())
            }
            _ => {
                warn!("Ignoring out-of-sequence camera enumeration PDU");
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, channel_id: u32) {
        if self.channel_id == Some(channel_id) {
            for device in &self.devices {
                device.advertised.store(false, Ordering::Release);
            }
            self.channel_id = None;
            self.state = EnumerationState::WaitingVersion;
            debug!("Camera enumeration channel closed");
        }
    }
}

impl DvcClientProcessor for EnumerationClient {}

/// Backend boundary for one explicitly selected camera.
///
/// Implementations manage platform capture resources.
/// Methods are synchronous and must not block indefinitely on device I/O.
pub trait CameraBackend: Send + 'static {
    fn activate(&mut self) -> Result<(), ErrorCode>;
    /// Deactivates the device.
    ///
    /// On error, the device must remain activated and retryable.
    fn deactivate(&mut self) -> Result<(), ErrorCode>;
    fn streams(&mut self) -> Result<Vec<StreamDescription>, ErrorCode>;
    fn media_types(&mut self, stream_index: u8) -> Result<Vec<MediaType>, ErrorCode>;
    fn current_media_type(&mut self, stream_index: u8) -> Result<MediaType, ErrorCode>;
    /// Atomically replaces the active stream selection.
    ///
    /// Captured samples are delivered through `sink`, which answers the server's Sample
    /// Requests. On error, any prior stream selection and its sink must remain active.
    fn start_streams(&mut self, streams: &[StartStreamInfo], sink: SampleSink) -> Result<(), ErrorCode>;
    /// Stops all active streams.
    ///
    /// On error, the current streams must remain active and retryable.
    fn stop_streams(&mut self) -> Result<(), ErrorCode>;
    /// Notifies the backend that the server requested one more sample on `stream_index`.
    ///
    /// The request is already recorded by the [`SampleSink`]; backends that only capture on
    /// demand can use this to wake their capture loop.
    fn sample_requested(&mut self, stream_index: u8) {
        let _ = stream_index;
    }
    /// Version 2 device properties.
    fn properties(&mut self) -> Result<Vec<PropertyDescription>, ErrorCode> {
        Ok(Vec::new())
    }
    /// Reads a property advertised by [`Self::properties`].
    fn property_value(
        &mut self,
        property_set: PropertySet,
        property_id: PropertyId,
    ) -> Result<PropertyValue, ErrorCode> {
        let _ = (property_set, property_id);
        Err(ErrorCode::ItemNotFound)
    }
    /// Writes a property advertised by [`Self::properties`].
    ///
    /// The value is already checked against the advertised description.
    fn set_property_value(
        &mut self,
        property_set: PropertySet,
        property_id: PropertyId,
        value: PropertyValue,
    ) -> Result<(), ErrorCode> {
        let _ = (property_set, property_id, value);
        Err(ErrorCode::OperationNotSupported)
    }
    /// Infallibly stops capture and releases device resources during channel teardown.
    fn shutdown(&mut self);
}

/// Result of offering a captured sample to a [`SampleSink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleDelivery {
    /// The sample answered an outstanding Sample Request.
    Sent,
    /// No Sample Request is outstanding; the backend may keep the sample for the next request.
    NotRequested,
    /// The streams were stopped or replaced; the sample is no longer wanted.
    Stopped,
    /// The sample does not match the stream's media type; a Sample Error Response was sent.
    Rejected,
    /// The session did not accept the message; the request stays outstanding.
    Failed,
}

struct SinkStream {
    stream_index: u8,
    media_type: MediaType,
    /// Channels of the unanswered Sample Requests, oldest first. Each response goes to the
    /// channel of the request it answers.
    requests: spin::Mutex<VecDeque<u32>>,
}

struct SampleSinkInner {
    version: ProtocolVersion,
    active: AtomicBool,
    streams: Vec<SinkStream>,
    uplink: SampleUplink,
}

/// Delivery handle for the streams selected by one Start Streams Request.
///
/// The handle tracks the server's outstanding Sample Requests and sends one Sample Response per
/// request. It may be cloned and used from capture threads. It stops delivering once the streams
/// are stopped, replaced, or the channel closes.
#[derive(Clone)]
pub struct SampleSink {
    inner: Arc<SampleSinkInner>,
}

impl core::fmt::Debug for SampleSink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SampleSink")
            .field("active", &self.is_active())
            .finish_non_exhaustive()
    }
}

impl SampleSink {
    fn new(version: ProtocolVersion, streams: &[StartStreamInfo], uplink: SampleUplink) -> Self {
        Self {
            inner: Arc::new(SampleSinkInner {
                version,
                active: AtomicBool::new(true),
                streams: streams
                    .iter()
                    .map(|stream| SinkStream {
                        stream_index: stream.stream_index,
                        media_type: stream.media_type,
                        requests: spin::Mutex::new(VecDeque::new()),
                    })
                    .collect(),
                uplink,
            }),
        }
    }

    /// Returns whether the streams of this sink are still selected.
    pub fn is_active(&self) -> bool {
        self.inner.active.load(Ordering::Acquire)
    }

    /// Number of unanswered Sample Requests for `stream_index`.
    pub fn pending_requests(&self, stream_index: u8) -> u32 {
        self.stream(stream_index).map_or(0, |stream| {
            u32::try_from(stream.requests.lock().len()).unwrap_or(u32::MAX)
        })
    }

    /// Answers one outstanding Sample Request with `sample`.
    pub fn send_sample(&self, stream_index: u8, sample: Vec<u8>) -> SampleDelivery {
        let Some(stream) = self.stream(stream_index) else {
            return SampleDelivery::Stopped;
        };
        if !stream.media_type.validate_sample(&sample) {
            return match self.deliver(
                stream,
                DevicePdu::SampleErrorResponse {
                    stream_index,
                    error: ErrorCode::InvalidMediaType,
                },
            ) {
                SampleDelivery::Sent => SampleDelivery::Rejected,
                other => other,
            };
        }
        self.deliver(stream, DevicePdu::SampleResponse { stream_index, sample })
    }

    /// Answers one outstanding Sample Request with a Sample Error Response.
    pub fn send_error(&self, stream_index: u8, error: ErrorCode) -> SampleDelivery {
        let Some(stream) = self.stream(stream_index) else {
            return SampleDelivery::Stopped;
        };
        self.deliver(stream, DevicePdu::SampleErrorResponse { stream_index, error })
    }

    fn stream(&self, stream_index: u8) -> Option<&SinkStream> {
        self.inner
            .streams
            .iter()
            .find(|stream| stream.stream_index == stream_index)
    }

    fn deliver(&self, stream: &SinkStream, pdu: DevicePdu) -> SampleDelivery {
        if !self.is_active() {
            return SampleDelivery::Stopped;
        }
        let Some(channel_id) = stream.requests.lock().pop_front() else {
            return SampleDelivery::NotRequested;
        };
        let message = DeviceMessage::new(self.inner.version, pdu);
        let result = encode_dvc_messages(channel_id, vec![Box::new(message)], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))
            .and_then(|messages| (self.inner.uplink)(channel_id, messages));
        match result {
            Ok(()) => SampleDelivery::Sent,
            Err(error) => {
                trace!(%error, "Camera sample was not accepted by the session");
                stream.requests.lock().push_front(channel_id);
                SampleDelivery::Failed
            }
        }
    }

    fn request(&self, stream_index: u8, channel_id: u32) -> Result<(), ErrorCode> {
        let stream = self.stream(stream_index).ok_or(ErrorCode::InvalidStreamNumber)?;
        let mut requests = stream.requests.lock();
        if MAX_PENDING_SAMPLE_REQUESTS <= u32::try_from(requests.len()).unwrap_or(u32::MAX) {
            return Err(ErrorCode::InvalidRequest);
        }
        requests.push_back(channel_id);
        Ok(())
    }

    fn stop(&self) {
        self.inner.active.store(false, Ordering::Release);
    }
}

/// State of one redirected camera, shared by every channel the server opens for it.
///
/// Windows opens several channels with the device's name at once, activating the device on one
/// and sending requests and Sample Requests on others, so activation and streaming belong to the
/// device rather than to a channel.
struct DeviceState<B> {
    /// Created when the first channel opens and kept for the device's lifetime.
    backend: Option<B>,
    activation_depth: u32,
    /// Activations held by each channel, as (channel id, count), summing to `activation_depth`.
    holders: Vec<(u32, u32)>,
    sink: Option<SampleSink>,
    open_channels: u32,
}

impl<B> DeviceState<B>
where
    B: CameraBackend,
{
    /// Stops capture and releases the device once nothing uses it.
    fn shutdown(&mut self) {
        let streaming = self.sink.take().inspect(SampleSink::stop).is_some();
        if streaming || self.activation_depth != 0 {
            if let Some(backend) = self.backend.as_mut() {
                backend.shutdown();
            }
        }
        self.activation_depth = 0;
        self.holders.clear();
    }

    fn activate(&mut self, channel_id: u32) {
        self.activation_depth += 1;
        match self.holders.iter_mut().find(|(holder, _)| *holder == channel_id) {
            Some((_, count)) => *count += 1,
            None => self.holders.push((channel_id, 1)),
        }
    }

    /// Releases one activation, preferring one held by `channel_id`.
    ///
    /// A channel may deactivate a device another channel activated; that activation then no
    /// longer belongs to the other channel.
    fn deactivate(&mut self, channel_id: u32) {
        self.activation_depth = self.activation_depth.saturating_sub(1);
        let position = self
            .holders
            .iter()
            .position(|(holder, _)| *holder == channel_id)
            .or_else(|| (!self.holders.is_empty()).then_some(0));
        if let Some(position) = position {
            self.holders[position].1 -= 1;
            if self.holders[position].1 == 0 {
                self.holders.swap_remove(position);
            }
        }
    }

    /// Returns the activations `channel_id` holds.
    fn held_by(&self, channel_id: u32) -> u32 {
        self.holders
            .iter()
            .find(|(holder, _)| *holder == channel_id)
            .map_or(0, |(_, count)| *count)
    }
}

type SharedDevice<B> = Arc<spin::Mutex<DeviceState<B>>>;

/// Per-channel RDPECAM client processor for one redirected camera.
pub struct DeviceClient<B> {
    channel_name: String,
    session: Arc<SessionShared>,
    advertised: Arc<AtomicBool>,
    device: SharedDevice<B>,
    uplink: SampleUplink,
    version: ProtocolVersion,
    channel_id: Option<u32>,
}

impl<B> DeviceClient<B>
where
    B: CameraBackend,
{
    /// Creates a processor for a device served by a single channel.
    ///
    /// [`RedirectedDevice::listener`] shares one device between all the channels the server opens.
    pub fn new(device: &RedirectedDevice, backend: B, uplink: SampleUplink) -> Self {
        let state = Arc::new(spin::Mutex::new(DeviceState {
            backend: Some(backend),
            activation_depth: 0,
            holders: Vec::new(),
            sink: None,
            open_channels: 0,
        }));
        Self::with_device(device, state, uplink)
    }

    fn with_device(device: &RedirectedDevice, state: SharedDevice<B>, uplink: SampleUplink) -> Self {
        Self {
            channel_name: device.descriptor.channel_name.clone(),
            session: Arc::clone(&device.session),
            advertised: Arc::clone(&device.advertised),
            device: state,
            uplink,
            version: ProtocolVersion::V1,
            channel_id: None,
        }
    }

    /// Returns whether the device is activated, on any of its channels.
    pub fn is_activated(&self) -> bool {
        self.device.lock().activation_depth != 0
    }

    /// Returns whether the device streams, on any of its channels.
    pub fn is_streaming(&self) -> bool {
        self.device.lock().sink.is_some()
    }

    /// Protocol version used on this device channel.
    pub fn version(&self) -> ProtocolVersion {
        self.version
    }

    /// Runs `f` with the device's backend, if it was created.
    ///
    /// `f` must not use this device's processors: the device stays locked while it runs.
    pub fn with_backend<R>(&self, f: impl FnOnce(&mut B) -> R) -> Option<R> {
        self.device.lock().backend.as_mut().map(f)
    }

    /// Handles one server request. `None` means the response is sent later.
    fn respond(&mut self, channel_id: u32, request: DevicePdu) -> Option<DevicePdu> {
        let device = Arc::clone(&self.device);
        let mut state = device.lock();
        let state = &mut *state;
        let Some(backend) = state.backend.as_mut() else {
            return Some(DevicePdu::ErrorResponse(ErrorCode::Unexpected));
        };
        if matches!(request, DevicePdu::ActivateDeviceRequest) {
            if state.activation_depth == u32::MAX {
                return Some(DevicePdu::ErrorResponse(ErrorCode::InvalidRequest));
            }
            if state.activation_depth == 0 {
                if let Err(error) = backend.activate() {
                    return Some(DevicePdu::ErrorResponse(error));
                }
            }
            state.activate(channel_id);
            return Some(DevicePdu::SuccessResponse);
        }
        if request.min_version() > self.version {
            return Some(DevicePdu::ErrorResponse(ErrorCode::InvalidMessage));
        }
        if state.activation_depth == 0 {
            if let DevicePdu::SampleRequest(stream_index) = request {
                return Some(DevicePdu::SampleErrorResponse {
                    stream_index,
                    error: ErrorCode::NotInitialized,
                });
            }
            return Some(DevicePdu::ErrorResponse(ErrorCode::NotInitialized));
        }
        let response = match request {
            DevicePdu::DeactivateDeviceRequest => {
                // Deactivation stops the streams, as in the Windows client.
                if let Err(error) = stop_streaming(backend, &mut state.sink) {
                    return Some(DevicePdu::ErrorResponse(error));
                }
                if state.activation_depth == 1 {
                    if let Err(error) = backend.deactivate() {
                        return Some(DevicePdu::ErrorResponse(error));
                    }
                }
                state.deactivate(channel_id);
                DevicePdu::SuccessResponse
            }
            DevicePdu::StreamListRequest => match backend.streams() {
                Ok(streams) if valid_stream_count(streams.len()) && streams.iter().all(stream_is_valid) => {
                    DevicePdu::StreamListResponse(streams)
                }
                Ok(_) => DevicePdu::ErrorResponse(ErrorCode::Unexpected),
                Err(error) => DevicePdu::ErrorResponse(error),
            },
            DevicePdu::MediaTypeListRequest(stream_index) => match validate_stream_index(backend, stream_index) {
                Err(error) => DevicePdu::ErrorResponse(error),
                Ok(()) => match backend.media_types(stream_index) {
                    Ok(media_types)
                        if valid_media_type_count(media_types.len()) && media_types.iter().all(media_type_is_valid) =>
                    {
                        DevicePdu::MediaTypeListResponse(media_types)
                    }
                    Ok(_) => DevicePdu::ErrorResponse(ErrorCode::InvalidMediaType),
                    Err(error) => DevicePdu::ErrorResponse(error),
                },
            },
            DevicePdu::CurrentMediaTypeRequest(stream_index) => match validate_stream_index(backend, stream_index) {
                Err(error) => DevicePdu::ErrorResponse(error),
                Ok(()) => match backend.current_media_type(stream_index) {
                    Ok(media_type) if media_type_is_valid(&media_type) => {
                        DevicePdu::CurrentMediaTypeResponse(media_type)
                    }
                    Ok(_) => DevicePdu::ErrorResponse(ErrorCode::InvalidMediaType),
                    Err(error) => DevicePdu::ErrorResponse(error),
                },
            },
            DevicePdu::StartStreamsRequest(streams) => match start_streams(backend, &streams) {
                Err(error) => DevicePdu::ErrorResponse(error),
                Ok(()) => {
                    let sink = SampleSink::new(self.version, &streams, Arc::clone(&self.uplink));
                    if let Err(error) = backend.start_streams(&streams, sink.clone()) {
                        sink.stop();
                        return Some(DevicePdu::ErrorResponse(error));
                    }
                    if let Some(previous) = state.sink.replace(sink) {
                        previous.stop();
                    }
                    DevicePdu::SuccessResponse
                }
            },
            DevicePdu::StopStreamsRequest => match stop_streaming(backend, &mut state.sink) {
                Ok(()) => DevicePdu::SuccessResponse,
                Err(error) => DevicePdu::ErrorResponse(error),
            },
            DevicePdu::SampleRequest(stream_index) => {
                let Some(sink) = &state.sink else {
                    return Some(DevicePdu::SampleErrorResponse {
                        stream_index,
                        error: ErrorCode::InvalidRequest,
                    });
                };
                // Samples answer on the channel that asked for them.
                return match sink.request(stream_index, channel_id) {
                    Ok(()) => {
                        backend.sample_requested(stream_index);
                        None
                    }
                    Err(error) => Some(DevicePdu::SampleErrorResponse { stream_index, error }),
                };
            }
            DevicePdu::PropertyListRequest => match backend.properties() {
                Ok(properties) if properties.len() <= MAX_PROPERTIES => DevicePdu::PropertyListResponse(properties),
                Ok(_) => DevicePdu::ErrorResponse(ErrorCode::Unexpected),
                Err(error) => DevicePdu::ErrorResponse(error),
            },
            DevicePdu::PropertyValueRequest {
                property_set,
                property_id,
            } => match find_property(backend, property_set, property_id)
                .and_then(|_| backend.property_value(property_set, property_id))
            {
                Ok(value) => DevicePdu::PropertyValueResponse(value),
                Err(error) => DevicePdu::ErrorResponse(error),
            },
            DevicePdu::SetPropertyValueRequest {
                property_set,
                property_id,
                value,
            } => match find_property(backend, property_set, property_id).and_then(|property| {
                if !property.accepts(value) {
                    return Err(ErrorCode::InvalidRequest);
                }
                backend.set_property_value(property_set, property_id, value)
            }) {
                Ok(()) => DevicePdu::SuccessResponse,
                Err(error) => DevicePdu::ErrorResponse(error),
            },
            _ => DevicePdu::ErrorResponse(ErrorCode::InvalidMessage),
        };
        Some(response)
    }

    /// Releases this channel's hold on the device.
    fn release(&mut self) {
        let Some(channel_id) = self.channel_id else {
            return;
        };
        let mut state = self.device.lock();
        state.open_channels = state.open_channels.saturating_sub(1);
        let held = state.held_by(channel_id);
        // Shutting down sees the activations being released, so the backend still learns about them.
        if state.activation_depth == held || state.open_channels == 0 {
            state.shutdown();
        } else {
            state.activation_depth -= held;
            state.holders.retain(|(holder, _)| *holder != channel_id);
        }
    }

    fn reply(&self, pdu: DevicePdu) -> Vec<DvcMessage> {
        vec![Box::new(DeviceMessage::new(self.version, pdu))]
    }
}

/// Validates a Start Streams Request against the streams and media types the backend offers.
fn start_streams<B: CameraBackend>(backend: &mut B, streams: &[StartStreamInfo]) -> Result<(), ErrorCode> {
    if !valid_stream_count(streams.len()) {
        return Err(ErrorCode::InvalidRequest);
    }
    let available_streams = match backend.streams() {
        Ok(available) if valid_stream_count(available.len()) && available.iter().all(stream_is_valid) => available,
        Ok(_) => return Err(ErrorCode::Unexpected),
        Err(error) => return Err(error),
    };
    for (index, stream) in streams.iter().enumerate() {
        if streams[..index]
            .iter()
            .any(|previous| previous.stream_index == stream.stream_index)
        {
            return Err(ErrorCode::InvalidRequest);
        }
        if usize::from(stream.stream_index) >= available_streams.len() {
            return Err(ErrorCode::InvalidStreamNumber);
        }
        let supported = backend.media_types(stream.stream_index)?;
        if !valid_media_type_count(supported.len())
            || !supported.iter().all(media_type_is_valid)
            || !supported.contains(&stream.media_type)
        {
            return Err(ErrorCode::InvalidMediaType);
        }
    }
    Ok(())
}

/// Stops capture and its sink, if streaming. On failure, the sink keeps streaming.
fn stop_streaming<B: CameraBackend>(backend: &mut B, sink: &mut Option<SampleSink>) -> Result<(), ErrorCode> {
    if sink.is_some() {
        backend.stop_streams()?;
        if let Some(sink) = sink.take() {
            sink.stop();
        }
    }
    Ok(())
}

fn find_property<B: CameraBackend>(
    backend: &mut B,
    property_set: PropertySet,
    property_id: PropertyId,
) -> Result<PropertyDescription, ErrorCode> {
    if !property_set.is_known() {
        return Err(ErrorCode::SetNotFound);
    }
    let properties = backend.properties()?;
    properties
        .into_iter()
        .find(|property| property.property_set == property_set && property.property_id == property_id)
        .ok_or(ErrorCode::ItemNotFound)
}

fn validate_stream_index<B: CameraBackend>(backend: &mut B, stream_index: u8) -> Result<(), ErrorCode> {
    let streams = backend.streams()?;
    if !valid_stream_count(streams.len()) || !streams.iter().all(stream_is_valid) {
        return Err(ErrorCode::Unexpected);
    }
    if usize::from(stream_index) >= streams.len() {
        return Err(ErrorCode::InvalidStreamNumber);
    }
    Ok(())
}

impl<B> AsAny for DeviceClient<B>
where
    B: CameraBackend,
{
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl<B> DvcProcessor for DeviceClient<B>
where
    B: CameraBackend,
{
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        if !self.advertised.load(Ordering::Acquire) {
            return Err(pdu_other_err!(
                "DeviceClient::start",
                "camera device was not negotiated and advertised"
            ));
        }
        if self.channel_id.is_some() {
            self.release();
        }
        self.version = self.session.version();
        self.channel_id = Some(channel_id);
        self.device.lock().open_channels += 1;
        debug!(channel_id, version = ?self.version, "Camera device channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if !self.advertised.load(Ordering::Acquire) {
            self.device.lock().shutdown();
            let response = match decode::<DeviceMessage>(payload) {
                Ok(request) if request.pdu.is_client_message() => return Ok(Vec::new()),
                Ok(DeviceMessage {
                    pdu: DevicePdu::SampleRequest(stream_index),
                    ..
                }) => DevicePdu::SampleErrorResponse {
                    stream_index,
                    error: ErrorCode::NotInitialized,
                },
                Ok(_) => DevicePdu::ErrorResponse(ErrorCode::NotInitialized),
                Err(_) => DevicePdu::ErrorResponse(ErrorCode::InvalidMessage),
            };
            return Ok(self.reply(response));
        }
        if self.channel_id != Some(channel_id) {
            return Err(pdu_other_err!(
                "DeviceClient::process",
                "camera device channel id does not match the active channel"
            ));
        }
        let response = match decode::<DeviceMessage>(payload) {
            Ok(request) if request.pdu.is_client_message() => {
                warn!("Ignoring client-originated camera response PDU received from server");
                return Ok(Vec::new());
            }
            Ok(request) => {
                if matches!(request.pdu, DevicePdu::SampleRequest(_)) {
                    trace!(channel_id, request = ?request.pdu, "Camera request");
                } else {
                    debug!(channel_id, request = ?request.pdu, "Camera request");
                }
                let response = if request.version != self.version {
                    // Every message on a device channel carries the negotiated version.
                    warn!(channel_id, version = ?request.version, "Rejecting camera request with the wrong version");
                    Some(match request.pdu {
                        DevicePdu::SampleRequest(stream_index) => DevicePdu::SampleErrorResponse {
                            stream_index,
                            error: ErrorCode::InvalidMessage,
                        },
                        _ => DevicePdu::ErrorResponse(ErrorCode::InvalidMessage),
                    })
                } else {
                    self.respond(channel_id, request.pdu)
                };
                match response {
                    Some(response) => {
                        debug!(channel_id, ?response, "Camera response");
                        response
                    }
                    None => return Ok(Vec::new()),
                }
            }
            Err(error) => {
                warn!(%error, "Rejecting malformed camera device PDU");
                DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
            }
        };
        Ok(self.reply(response))
    }

    fn close(&mut self, channel_id: u32) {
        if self.channel_id == Some(channel_id) {
            self.release();
            self.channel_id = None;
            debug!(channel_id, "Camera device channel closed");
        }
    }
}

impl<B> DvcClientProcessor for DeviceClient<B> where B: CameraBackend {}

/// DVC listener for an explicitly selected camera.
///
/// Every channel the server opens with the device's name shares one device state and one backend,
/// created by the factory when the first channel opens.
pub struct DeviceChannelListener<B> {
    device: RedirectedDevice,
    state: SharedDevice<B>,
    factory: Box<dyn FnMut() -> Option<B> + Send>,
    uplink: SampleUplink,
}

impl<B> DvcChannelListener for DeviceChannelListener<B>
where
    B: CameraBackend,
{
    fn channel_name(&self) -> &str {
        &self.device.descriptor.channel_name
    }

    fn create(&mut self, _channel_id: u32) -> Option<Box<dyn DvcClientProcessor>> {
        if !self.device.advertised.load(Ordering::Acquire) {
            return None;
        }
        {
            let mut state = self.state.lock();
            if state.backend.is_none() {
                state.backend = Some((self.factory)()?);
            }
        }
        Some(Box::new(DeviceClient::with_device(
            &self.device,
            Arc::clone(&self.state),
            Arc::clone(&self.uplink),
        )))
    }

    fn is_available(&self) -> bool {
        self.device.advertised.load(Ordering::Acquire)
    }
}

fn valid_stream_count(count: usize) -> bool {
    (1..=usize::from(u8::MAX)).contains(&count)
}

fn valid_media_type_count(count: usize) -> bool {
    (1..=crate::pdu::MAX_MEDIA_TYPES).contains(&count)
}

fn stream_is_valid(stream: &StreamDescription) -> bool {
    stream.frame_source_types != 0
        && stream.frame_source_types
            & !(StreamDescription::COLOR | StreamDescription::INFRARED | StreamDescription::CUSTOM)
            == 0
}

fn media_type_is_valid(media_type: &MediaType) -> bool {
    media_type.flags & !(MediaType::DECODING_REQUIRED | MediaType::BOTTOM_UP_IMAGE) == 0
        && media_type.validate_for_backend().is_ok()
}
