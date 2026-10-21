//! Client state machines for the MS-RDPECAM dynamic virtual channels.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

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

    /// Protocol version selected by the server, once negotiated.
    pub fn version(&self) -> Option<ProtocolVersion> {
        self.ready().then(|| self.session.version())
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
    pending_requests: AtomicU32,
}

struct SampleSinkInner {
    channel_id: u32,
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
            .field("channel_id", &self.inner.channel_id)
            .field("active", &self.is_active())
            .finish_non_exhaustive()
    }
}

impl SampleSink {
    fn new(channel_id: u32, version: ProtocolVersion, streams: &[StartStreamInfo], uplink: SampleUplink) -> Self {
        Self {
            inner: Arc::new(SampleSinkInner {
                channel_id,
                version,
                active: AtomicBool::new(true),
                streams: streams
                    .iter()
                    .map(|stream| SinkStream {
                        stream_index: stream.stream_index,
                        media_type: stream.media_type,
                        pending_requests: AtomicU32::new(0),
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

    /// Media type selected for `stream_index`.
    pub fn media_type(&self, stream_index: u8) -> Option<MediaType> {
        self.stream(stream_index).map(|stream| stream.media_type)
    }

    /// Number of unanswered Sample Requests for `stream_index`.
    pub fn pending_requests(&self, stream_index: u8) -> u32 {
        self.stream(stream_index)
            .map_or(0, |stream| stream.pending_requests.load(Ordering::Acquire))
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
        if stream
            .pending_requests
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| pending.checked_sub(1))
            .is_err()
        {
            return SampleDelivery::NotRequested;
        }
        let message = DeviceMessage::new(self.inner.version, pdu);
        let result = encode_dvc_messages(self.inner.channel_id, vec![Box::new(message)], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))
            .and_then(|messages| (self.inner.uplink)(self.inner.channel_id, messages));
        match result {
            Ok(()) => SampleDelivery::Sent,
            Err(error) => {
                trace!(%error, "Camera sample was not accepted by the session");
                stream.pending_requests.fetch_add(1, Ordering::AcqRel);
                SampleDelivery::Failed
            }
        }
    }

    fn request(&self, stream_index: u8) -> Result<(), ErrorCode> {
        let stream = self.stream(stream_index).ok_or(ErrorCode::InvalidStreamNumber)?;
        stream
            .pending_requests
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                (pending < MAX_PENDING_SAMPLE_REQUESTS).then_some(pending + 1)
            })
            .map(|_| ())
            .map_err(|_| ErrorCode::InvalidRequest)
    }

    fn stop(&self) {
        self.inner.active.store(false, Ordering::Release);
    }
}

/// Per-device RDPECAM client processor.
pub struct DeviceClient<B> {
    channel_name: String,
    session: Arc<SessionShared>,
    advertised: Arc<AtomicBool>,
    backend: B,
    uplink: SampleUplink,
    version: ProtocolVersion,
    channel_id: Option<u32>,
    activation_depth: u32,
    sink: Option<SampleSink>,
}

impl<B> DeviceClient<B>
where
    B: CameraBackend,
{
    pub fn new(device: &RedirectedDevice, backend: B, uplink: SampleUplink) -> Self {
        Self {
            channel_name: device.descriptor.channel_name.clone(),
            session: Arc::clone(&device.session),
            advertised: Arc::clone(&device.advertised),
            backend,
            uplink,
            version: ProtocolVersion::V1,
            channel_id: None,
            activation_depth: 0,
            sink: None,
        }
    }

    pub fn is_activated(&self) -> bool {
        self.activation_depth != 0
    }

    pub fn is_streaming(&self) -> bool {
        self.sink.is_some()
    }

    /// Protocol version used on this device channel.
    pub fn version(&self) -> ProtocolVersion {
        self.version
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Handles one server request. `None` means the response is sent later.
    fn respond(&mut self, request: DevicePdu) -> Option<DevicePdu> {
        if matches!(request, DevicePdu::ActivateDeviceRequest) {
            return Some(self.activate());
        }
        if request.min_version() > self.version {
            return Some(DevicePdu::ErrorResponse(ErrorCode::InvalidMessage));
        }
        if self.activation_depth == 0 {
            if let DevicePdu::SampleRequest(stream_index) = request {
                return Some(DevicePdu::SampleErrorResponse {
                    stream_index,
                    error: ErrorCode::NotInitialized,
                });
            }
            return Some(DevicePdu::ErrorResponse(ErrorCode::NotInitialized));
        }
        let response = match request {
            DevicePdu::DeactivateDeviceRequest => self.deactivate(),
            DevicePdu::StreamListRequest => match self.backend.streams() {
                Ok(streams) if valid_stream_count(streams.len()) && streams.iter().all(stream_is_valid) => {
                    DevicePdu::StreamListResponse(streams)
                }
                Ok(_) => DevicePdu::ErrorResponse(ErrorCode::Unexpected),
                Err(error) => DevicePdu::ErrorResponse(error),
            },
            DevicePdu::MediaTypeListRequest(stream_index) => match self.validate_stream_index(stream_index) {
                Err(error) => DevicePdu::ErrorResponse(error),
                Ok(()) => match self.backend.media_types(stream_index) {
                    Ok(media_types)
                        if valid_media_type_count(media_types.len()) && media_types.iter().all(media_type_is_valid) =>
                    {
                        DevicePdu::MediaTypeListResponse(media_types)
                    }
                    Ok(_) => DevicePdu::ErrorResponse(ErrorCode::InvalidMediaType),
                    Err(error) => DevicePdu::ErrorResponse(error),
                },
            },
            DevicePdu::CurrentMediaTypeRequest(stream_index) => match self.validate_stream_index(stream_index) {
                Err(error) => DevicePdu::ErrorResponse(error),
                Ok(()) => match self.backend.current_media_type(stream_index) {
                    Ok(media_type) if media_type_is_valid(&media_type) => {
                        DevicePdu::CurrentMediaTypeResponse(media_type)
                    }
                    Ok(_) => DevicePdu::ErrorResponse(ErrorCode::InvalidMediaType),
                    Err(error) => DevicePdu::ErrorResponse(error),
                },
            },
            DevicePdu::StartStreamsRequest(streams) => self.start_streams(streams),
            DevicePdu::StopStreamsRequest => self.stop_streams(),
            DevicePdu::SampleRequest(stream_index) => return self.sample_request(stream_index),
            DevicePdu::PropertyListRequest => match self.backend.properties() {
                Ok(properties) if properties.len() <= MAX_PROPERTIES => DevicePdu::PropertyListResponse(properties),
                Ok(_) => DevicePdu::ErrorResponse(ErrorCode::Unexpected),
                Err(error) => DevicePdu::ErrorResponse(error),
            },
            DevicePdu::PropertyValueRequest {
                property_set,
                property_id,
            } => match self
                .find_property(property_set, property_id)
                .and_then(|_| self.backend.property_value(property_set, property_id))
            {
                Ok(value) => DevicePdu::PropertyValueResponse(value),
                Err(error) => DevicePdu::ErrorResponse(error),
            },
            DevicePdu::SetPropertyValueRequest {
                property_set,
                property_id,
                value,
            } => match self.find_property(property_set, property_id).and_then(|property| {
                if !property.accepts(value) {
                    return Err(ErrorCode::InvalidRequest);
                }
                self.backend.set_property_value(property_set, property_id, value)
            }) {
                Ok(()) => DevicePdu::SuccessResponse,
                Err(error) => DevicePdu::ErrorResponse(error),
            },
            _ => DevicePdu::ErrorResponse(ErrorCode::InvalidMessage),
        };
        Some(response)
    }

    fn activate(&mut self) -> DevicePdu {
        if self.activation_depth == u32::MAX {
            return DevicePdu::ErrorResponse(ErrorCode::InvalidRequest);
        }
        if self.activation_depth == 0 {
            if let Err(error) = self.backend.activate() {
                return DevicePdu::ErrorResponse(error);
            }
        }
        self.activation_depth += 1;
        DevicePdu::SuccessResponse
    }

    fn deactivate(&mut self) -> DevicePdu {
        if let Some(sink) = &self.sink {
            if let Err(error) = self.backend.stop_streams() {
                return DevicePdu::ErrorResponse(error);
            }
            sink.stop();
            self.sink = None;
        }
        if self.activation_depth == 1 {
            if let Err(error) = self.backend.deactivate() {
                return DevicePdu::ErrorResponse(error);
            }
        }
        self.activation_depth -= 1;
        DevicePdu::SuccessResponse
    }

    fn start_streams(&mut self, streams: Vec<StartStreamInfo>) -> DevicePdu {
        let Some(channel_id) = self.channel_id else {
            return DevicePdu::ErrorResponse(ErrorCode::Unexpected);
        };
        if !valid_stream_count(streams.len()) {
            return DevicePdu::ErrorResponse(ErrorCode::InvalidRequest);
        }
        let available_streams = match self.backend.streams() {
            Ok(available) if valid_stream_count(available.len()) && available.iter().all(stream_is_valid) => available,
            Ok(_) => return DevicePdu::ErrorResponse(ErrorCode::Unexpected),
            Err(error) => return DevicePdu::ErrorResponse(error),
        };
        for (index, stream) in streams.iter().enumerate() {
            if streams[..index]
                .iter()
                .any(|previous| previous.stream_index == stream.stream_index)
            {
                return DevicePdu::ErrorResponse(ErrorCode::InvalidRequest);
            }
            if usize::from(stream.stream_index) >= available_streams.len() {
                return DevicePdu::ErrorResponse(ErrorCode::InvalidStreamNumber);
            }
            let supported = match self.backend.media_types(stream.stream_index) {
                Ok(supported) => supported,
                Err(error) => return DevicePdu::ErrorResponse(error),
            };
            if !valid_media_type_count(supported.len())
                || !supported.iter().all(media_type_is_valid)
                || !supported.contains(&stream.media_type)
            {
                return DevicePdu::ErrorResponse(ErrorCode::InvalidMediaType);
            }
        }
        let sink = SampleSink::new(channel_id, self.version, &streams, Arc::clone(&self.uplink));
        if let Err(error) = self.backend.start_streams(&streams, sink.clone()) {
            sink.stop();
            return DevicePdu::ErrorResponse(error);
        }
        if let Some(previous) = self.sink.replace(sink) {
            previous.stop();
        }
        DevicePdu::SuccessResponse
    }

    fn stop_streams(&mut self) -> DevicePdu {
        let Some(sink) = &self.sink else {
            return DevicePdu::SuccessResponse;
        };
        if let Err(error) = self.backend.stop_streams() {
            return DevicePdu::ErrorResponse(error);
        }
        sink.stop();
        self.sink = None;
        DevicePdu::SuccessResponse
    }

    fn sample_request(&mut self, stream_index: u8) -> Option<DevicePdu> {
        let Some(sink) = &self.sink else {
            return Some(DevicePdu::SampleErrorResponse {
                stream_index,
                error: ErrorCode::InvalidRequest,
            });
        };
        match sink.request(stream_index) {
            Ok(()) => {
                self.backend.sample_requested(stream_index);
                None
            }
            Err(error) => Some(DevicePdu::SampleErrorResponse { stream_index, error }),
        }
    }

    fn find_property(
        &mut self,
        property_set: PropertySet,
        property_id: PropertyId,
    ) -> Result<PropertyDescription, ErrorCode> {
        if !property_set.is_known() {
            return Err(ErrorCode::SetNotFound);
        }
        let properties = self.backend.properties()?;
        properties
            .into_iter()
            .find(|property| property.property_set == property_set && property.property_id == property_id)
            .ok_or(ErrorCode::ItemNotFound)
    }

    fn validate_stream_index(&mut self, stream_index: u8) -> Result<(), ErrorCode> {
        let streams = self.backend.streams()?;
        if !valid_stream_count(streams.len()) || !streams.iter().all(stream_is_valid) {
            return Err(ErrorCode::Unexpected);
        }
        if usize::from(stream_index) >= streams.len() {
            return Err(ErrorCode::InvalidStreamNumber);
        }
        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
            self.backend.shutdown();
        } else if self.activation_depth != 0 {
            self.backend.shutdown();
        }
        self.activation_depth = 0;
    }

    fn reply(&self, pdu: DevicePdu) -> Vec<DvcMessage> {
        vec![Box::new(DeviceMessage::new(self.version, pdu))]
    }
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
        self.shutdown();
        self.version = self.session.version();
        self.channel_id = Some(channel_id);
        debug!(channel_id, version = ?self.version, "Camera device channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if !self.advertised.load(Ordering::Acquire) {
            self.shutdown();
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
            Ok(request) => match self.respond(request.pdu) {
                Some(response) => response,
                None => return Ok(Vec::new()),
            },
            Err(error) => {
                warn!(%error, "Rejecting malformed camera device PDU");
                DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
            }
        };
        Ok(self.reply(response))
    }

    fn close(&mut self, channel_id: u32) {
        if self.channel_id == Some(channel_id) {
            self.shutdown();
            self.channel_id = None;
            debug!("Camera device channel closed");
        }
    }
}

impl<B> DvcClientProcessor for DeviceClient<B> where B: CameraBackend {}

/// DVC listener that creates a client for an explicitly selected camera.
///
/// A new backend is requested from the factory each time the server opens the device channel.
pub struct DeviceChannelListener<B> {
    device: RedirectedDevice,
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
        let backend = (self.factory)()?;
        Some(Box::new(DeviceClient::new(
            &self.device,
            backend,
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
