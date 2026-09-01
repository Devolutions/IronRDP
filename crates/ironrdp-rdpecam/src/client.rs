//! Client state machines for the MS-RDPECAM dynamic virtual channels.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use ironrdp_core::{AsAny, decode, impl_as_any, invalid_field_err};
use ironrdp_dvc::{DvcChannelListener, DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use crate::ENUMERATION_CHANNEL_NAME;
use crate::pdu::{
    DeviceDescriptor, DevicePdu, EnumerationPdu, ErrorCode, MediaType, ProtocolVersion, StartStreamInfo,
    StreamDescription,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnumerationState {
    WaitingVersion,
    Ready,
}

/// Shared redirection gate for an enumeration channel and its selected devices.
#[derive(Debug, Clone)]
pub struct CameraRedirectionSession {
    identity: Arc<()>,
}

impl CameraRedirectionSession {
    pub fn new() -> Self {
        Self { identity: Arc::new(()) }
    }

    /// Creates one camera selection that may be advertised by this session.
    pub fn select_device(&self, descriptor: DeviceDescriptor) -> ironrdp_core::EncodeResult<RedirectedDevice> {
        descriptor.validate()?;
        Ok(RedirectedDevice {
            descriptor,
            session_identity: Arc::clone(&self.identity),
            advertised: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn enumeration_client(&self, devices: Vec<RedirectedDevice>) -> ironrdp_core::EncodeResult<EnumerationClient> {
        EnumerationClient::new(Arc::clone(&self.identity), devices)
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
    session_identity: Arc<()>,
    advertised: Arc<AtomicBool>,
}

impl RedirectedDevice {
    pub fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    pub fn listener<B>(&self, backend: B) -> DeviceChannelListener<B>
    where
        B: CameraBackend,
    {
        DeviceChannelListener {
            channel_name: self.descriptor.channel_name.clone(),
            advertised: Arc::clone(&self.advertised),
            backend: Some(backend),
        }
    }
}

/// Client processor for the RDPECAM device-enumeration channel.
///
/// Only descriptors explicitly supplied by the caller are advertised.
#[derive(Debug)]
pub struct EnumerationClient {
    session_identity: Arc<()>,
    devices: Vec<RedirectedDevice>,
    state: EnumerationState,
    channel_id: Option<u32>,
}

impl EnumerationClient {
    fn new(session_identity: Arc<()>, devices: Vec<RedirectedDevice>) -> ironrdp_core::EncodeResult<Self> {
        if devices.len() > usize::from(u8::MAX) {
            return Err(invalid_field_err!(
                "devices",
                "at most 255 redirected cameras may be configured"
            ));
        }
        for (index, device) in devices.iter().enumerate() {
            device.descriptor.validate()?;
            if !Arc::ptr_eq(&session_identity, &device.session_identity) {
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
            session_identity,
            devices,
            state: EnumerationState::WaitingVersion,
            channel_id: None,
        })
    }

    pub fn ready(&self) -> bool {
        self.state == EnumerationState::Ready
    }

    /// Advertises a newly selected camera after version negotiation.
    pub fn add_device(&mut self, device: RedirectedDevice) -> PduResult<Vec<SvcMessage>> {
        let channel_id = self.active_channel_id("EnumerationClient::add_device")?;
        if !Arc::ptr_eq(&self.session_identity, &device.session_identity) {
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
        let pdu = EnumerationPdu::DeviceAdded {
            version: ProtocolVersion::V1,
            device: device.descriptor.clone(),
        };
        let messages = encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))?;
        device.advertised.store(true, Ordering::Release);
        self.devices.push(device);
        Ok(messages)
    }

    /// Removes an advertised camera without activating or touching another device.
    pub fn remove_device(&mut self, channel_name: &str) -> PduResult<Vec<SvcMessage>> {
        let channel_id = self.active_channel_id("EnumerationClient::remove_device")?;
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
        let pdu = EnumerationPdu::DeviceRemoved {
            version: ProtocolVersion::V1,
            channel_name: String::from(channel_name),
        };
        let messages = encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))?;
        self.devices.remove(index);
        Ok(messages)
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
        self.channel_id = Some(channel_id);
        self.state = EnumerationState::WaitingVersion;
        debug!(channel_id, "Camera enumeration channel started");
        Ok(vec![Box::new(EnumerationPdu::SelectVersionRequest(
            ProtocolVersion::V1,
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
            EnumerationPdu::SelectVersionResponse(ProtocolVersion::V1)
                if self.state == EnumerationState::WaitingVersion =>
            {
                for device in &self.devices {
                    device.advertised.store(true, Ordering::Release);
                }
                self.state = EnumerationState::Ready;
                debug!("Camera protocol version negotiated");
                Ok(self
                    .devices
                    .iter()
                    .map(|device| -> DvcMessage {
                        Box::new(EnumerationPdu::DeviceAdded {
                            version: ProtocolVersion::V1,
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
    fn deactivate(&mut self) -> Result<(), ErrorCode>;
    fn streams(&mut self) -> Result<Vec<StreamDescription>, ErrorCode>;
    fn media_types(&mut self, stream_index: u8) -> Result<Vec<MediaType>, ErrorCode>;
    fn current_media_type(&mut self, stream_index: u8) -> Result<MediaType, ErrorCode>;
    /// Atomically replaces the active stream selection.
    ///
    /// On error, any prior stream selection must remain active.
    fn start_streams(&mut self, streams: &[StartStreamInfo]) -> Result<(), ErrorCode>;
    fn stop_streams(&mut self) -> Result<(), ErrorCode>;
    fn sample(&mut self, stream_index: u8) -> Result<Vec<u8>, ErrorCode>;
}

/// Per-device RDPECAM client processor.
pub struct DeviceClient<B> {
    channel_name: String,
    advertised: Arc<AtomicBool>,
    backend: B,
    channel_id: Option<u32>,
    activation_depth: u32,
    streaming: Option<Vec<StartStreamInfo>>,
}

impl<B> DeviceClient<B>
where
    B: CameraBackend,
{
    pub fn new(device: &RedirectedDevice, backend: B) -> Self {
        Self {
            channel_name: device.descriptor.channel_name.clone(),
            advertised: Arc::clone(&device.advertised),
            backend,
            channel_id: None,
            activation_depth: 0,
            streaming: None,
        }
    }

    pub fn is_activated(&self) -> bool {
        self.activation_depth != 0
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming.is_some()
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    fn respond(&mut self, request: DevicePdu) -> DevicePdu {
        if matches!(request, DevicePdu::ActivateDeviceRequest) {
            return self.activate();
        }
        if self.activation_depth == 0 {
            if let DevicePdu::SampleRequest(stream_index) = request {
                return DevicePdu::SampleErrorResponse {
                    stream_index,
                    error: ErrorCode::NotInitialized,
                };
            }
            return DevicePdu::ErrorResponse(ErrorCode::NotInitialized);
        }
        match request {
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
            DevicePdu::SampleRequest(stream_index) => self.sample(stream_index),
            _ => DevicePdu::ErrorResponse(ErrorCode::InvalidMessage),
        }
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
        if self.streaming.is_some() {
            if let Err(error) = self.backend.stop_streams() {
                return DevicePdu::ErrorResponse(error);
            }
            self.streaming = None;
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
        if let Err(error) = self.backend.start_streams(&streams) {
            return DevicePdu::ErrorResponse(error);
        }
        self.streaming = Some(streams);
        DevicePdu::SuccessResponse
    }

    fn stop_streams(&mut self) -> DevicePdu {
        if self.streaming.is_none() {
            return DevicePdu::SuccessResponse;
        }
        if let Err(error) = self.backend.stop_streams() {
            return DevicePdu::ErrorResponse(error);
        }
        self.streaming = None;
        DevicePdu::SuccessResponse
    }

    fn sample(&mut self, stream_index: u8) -> DevicePdu {
        let Some(media_type) = self.streaming.as_ref().and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream.stream_index == stream_index)
                .map(|stream| stream.media_type)
        }) else {
            return DevicePdu::SampleErrorResponse {
                stream_index,
                error: if self.streaming.is_some() {
                    ErrorCode::InvalidStreamNumber
                } else {
                    ErrorCode::InvalidRequest
                },
            };
        };
        match self.backend.sample(stream_index) {
            Ok(sample) if media_type.validate_sample(&sample) => DevicePdu::SampleResponse { stream_index, sample },
            Ok(_) => DevicePdu::SampleErrorResponse {
                stream_index,
                error: ErrorCode::InvalidMediaType,
            },
            Err(error) => DevicePdu::SampleErrorResponse { stream_index, error },
        }
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
        if self.streaming.take().is_some()
            && let Err(error) = self.backend.stop_streams()
        {
            warn!(?error, "Camera backend failed to stop while closing channel");
        }
        if self.activation_depth != 0
            && let Err(error) = self.backend.deactivate()
        {
            warn!(?error, "Camera backend failed to deactivate while closing channel");
        }
        self.activation_depth = 0;
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
        self.channel_id = Some(channel_id);
        debug!(channel_id, "Camera device channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if !self.advertised.load(Ordering::Acquire) {
            self.shutdown();
            let response = match decode(payload) {
                Ok(request) if is_client_response(&request) => return Ok(Vec::new()),
                Ok(DevicePdu::SampleRequest(stream_index)) => DevicePdu::SampleErrorResponse {
                    stream_index,
                    error: ErrorCode::NotInitialized,
                },
                Ok(_) => DevicePdu::ErrorResponse(ErrorCode::NotInitialized),
                Err(_) => DevicePdu::ErrorResponse(ErrorCode::InvalidMessage),
            };
            return Ok(vec![Box::new(response)]);
        }
        if self.channel_id != Some(channel_id) {
            return Err(pdu_other_err!(
                "DeviceClient::process",
                "camera device channel id does not match the active channel"
            ));
        }
        let response = match decode(payload) {
            Ok(request) if is_client_response(&request) => {
                warn!("Ignoring client-originated camera response PDU received from server");
                return Ok(Vec::new());
            }
            Ok(request) => self.respond(request),
            Err(error) => {
                warn!(%error, "Rejecting malformed camera device PDU");
                DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
            }
        };
        Ok(vec![Box::new(response)])
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

/// One-use DVC listener that creates a client for an explicitly selected camera.
pub struct DeviceChannelListener<B> {
    channel_name: String,
    advertised: Arc<AtomicBool>,
    backend: Option<B>,
}

impl<B> DvcChannelListener for DeviceChannelListener<B>
where
    B: CameraBackend,
{
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn create(&mut self, _channel_id: u32) -> Option<Box<dyn DvcClientProcessor>> {
        if !self.advertised.load(Ordering::Acquire) {
            return None;
        }
        let backend = self.backend.take()?;
        Some(Box::new(DeviceClient {
            channel_name: self.channel_name.clone(),
            advertised: Arc::clone(&self.advertised),
            backend,
            channel_id: None,
            activation_depth: 0,
            streaming: None,
        }))
    }

    fn is_available(&self) -> bool {
        self.backend.is_some()
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
    if matches!(media_type.format, crate::MediaFormat::H264 | crate::MediaFormat::Mjpg) {
        return false;
    }
    media_type.flags & !(MediaType::DECODING_REQUIRED | MediaType::BOTTOM_UP_IMAGE) == 0
        && media_type.validate_for_backend().is_ok()
}

fn is_client_response(pdu: &DevicePdu) -> bool {
    matches!(
        pdu,
        DevicePdu::SuccessResponse
            | DevicePdu::ErrorResponse(_)
            | DevicePdu::StreamListResponse(_)
            | DevicePdu::MediaTypeListResponse(_)
            | DevicePdu::CurrentMediaTypeResponse(_)
            | DevicePdu::SampleResponse { .. }
            | DevicePdu::SampleErrorResponse { .. }
    )
}
