//! Server-side processing for the Video Capture Virtual Channel Extension ([MS-RDPECAM]).
//!
//! [MS-RDPECAM]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::PduResult;
use tracing::{debug, warn};

use crate::ENUMERATION_CHANNEL_NAME;
use crate::pdu::{
    DeviceDescriptor, DeviceMessage, DevicePdu, EnumerationPdu, MAX_SAMPLE_SIZE, MediaFormat, MediaType,
    ProtocolVersion, StartStreamInfo, StreamDescription,
};

const MAX_ENUMERATION_MESSAGE_SIZE: usize = 64 * 1024;
const MAX_DEVICE_MESSAGE_SIZE: usize = MAX_SAMPLE_SIZE + 64 /* header headroom */;

/// Receives validated camera samples and their negotiated media type.
pub type CameraFrameCallback = Box<dyn FnMut(MediaType, &[u8]) + Send>;

/// Requested camera output and maximum dimensions the consumer can accept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesiredCameraFormat {
    pub width: u32,
    pub height: u32,
    pub frame_rate: f32,
    /// Maximum frame width accepted by the consumer.
    pub max_width: u32,
    /// Maximum frame height accepted by the consumer.
    pub max_height: u32,
}

/// Server-side processor for the camera device-enumeration channel ([2.1] "Transport").
///
/// Negotiates the protocol version ([3.3.5.1] "Processing a Select Version Request") and
/// forwards device notifications ([3.3.5.3] "Processing a Device Added Notification",
/// [3.3.5.4] "Processing a Device Removed Notification").
///
/// [2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/340f6508-5834-4cbf-a7fa-933cb91e319c
/// [3.3.5.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/aacadfc0-74c6-405c-9b42-5960f3e5e3e1
/// [3.3.5.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/3a915165-1cb5-4626-8f6a-4a868f0416be
/// [3.3.5.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/a30c4f18-5b03-4c3e-9103-c60ac4ddeb73
pub struct EnumerationServer {
    state: EnumerationState,
    on_device_added: Box<dyn FnMut(ProtocolVersion, DeviceDescriptor) + Send>,
    on_device_removed: Box<dyn FnMut(ProtocolVersion, String) + Send>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnumerationState {
    AwaitVersion,
    Ready(ProtocolVersion),
}

impl EnumerationServer {
    pub fn new(
        on_device_added: Box<dyn FnMut(ProtocolVersion, DeviceDescriptor) + Send>,
        on_device_removed: Box<dyn FnMut(ProtocolVersion, String) + Send>,
    ) -> Self {
        Self {
            state: EnumerationState::AwaitVersion,
            on_device_added,
            on_device_removed,
        }
    }
}

impl_as_any!(EnumerationServer);

impl DvcProcessor for EnumerationServer {
    fn channel_name(&self) -> &str {
        ENUMERATION_CHANNEL_NAME
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(MAX_ENUMERATION_MESSAGE_SIZE)
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.state = EnumerationState::AwaitVersion;
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: EnumerationPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring malformed camera enumeration PDU");
                return Ok(Vec::new());
            }
        };
        match (self.state, pdu) {
            (EnumerationState::AwaitVersion, EnumerationPdu::SelectVersionRequest(version)) => {
                let version = version.min(ProtocolVersion::MAX);
                self.state = EnumerationState::Ready(version);
                debug!(?version, "Camera protocol version negotiated");
                Ok(vec![Box::new(EnumerationPdu::SelectVersionResponse(version))])
            }
            (
                EnumerationState::Ready(version),
                EnumerationPdu::DeviceAdded {
                    version: received,
                    device,
                },
            ) if received == version => {
                (self.on_device_added)(version, device);
                Ok(Vec::new())
            }
            (
                EnumerationState::Ready(version),
                EnumerationPdu::DeviceRemoved {
                    version: received,
                    channel_name,
                },
            ) if received == version => {
                (self.on_device_removed)(version, channel_name);
                Ok(Vec::new())
            }
            _ => {
                warn!("Ignoring out-of-sequence camera enumeration PDU");
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        self.state = EnumerationState::AwaitVersion;
    }
}

impl DvcServerProcessor for EnumerationServer {}

/// Server-side processor for one redirected camera device channel ([1.3.4] "Device Initialization sequence", [1.3.5] "Video Capture sequence").
///
/// [1.3.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/606dc554-5d76-4f4a-8b0a-a379d57fffc3
/// [1.3.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/d7ae9bda-0dde-455b-82ef-e7858a87e46e
pub struct DeviceServer {
    channel_name: String,
    version: ProtocolVersion,
    desired: DesiredCameraFormat,
    allow_h264: bool,
    state: DeviceState,
    stop_requested: bool,
    stream_index: Option<u8>,
    media_type: Option<MediaType>,
    on_frame: CameraFrameCallback,
    on_failure: Box<dyn FnMut() + Send>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceState {
    AwaitActivation,
    AwaitStreams,
    AwaitMediaTypes,
    AwaitStart,
    Streaming,
    AwaitStopStreams,
    AwaitDeactivate,
    AwaitFailureStopStreams,
    AwaitFailureDeactivate,
    Stopped,
    Failed,
}

impl DeviceServer {
    pub fn new(
        device: &DeviceDescriptor,
        version: ProtocolVersion,
        desired: DesiredCameraFormat,
        on_frame: CameraFrameCallback,
    ) -> Self {
        Self::with_failure_handler(device, version, desired, on_frame, Box::new(|| {}))
    }

    /// Creates a server processor that notifies the caller after cleaning up camera setup or sampling failures.
    pub fn with_failure_handler(
        device: &DeviceDescriptor,
        version: ProtocolVersion,
        desired: DesiredCameraFormat,
        on_frame: CameraFrameCallback,
        on_failure: Box<dyn FnMut() + Send>,
    ) -> Self {
        Self {
            channel_name: device.channel_name.clone(),
            version,
            desired,
            allow_h264: false,
            state: DeviceState::AwaitActivation,
            stop_requested: false,
            stream_index: None,
            media_type: None,
            on_frame,
            on_failure,
        }
    }

    /// Allows selecting [`MediaFormat::H264`] ([2.2.3.8.1] "MEDIA_TYPE_DESCRIPTION") when the caller provides an H.264 decoder.
    ///
    /// [2.2.3.8.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/55082483-46ea-4290-8e8f-e56647c09b97
    #[must_use]
    pub fn with_h264(mut self, allow_h264: bool) -> Self {
        self.allow_h264 = allow_h264;
        self
    }

    fn message(&self, pdu: DevicePdu) -> DvcMessage {
        Box::new(DeviceMessage::new(self.version, pdu))
    }

    /// Begins stopping this device after capture ends ([1.3.5] "Video Capture sequence").
    ///
    /// If streaming has started, returns a Stop Streams Request ([3.3.5.15] "Sending a Stop Streams Request").
    /// Otherwise, defers Deactivate Device Request ([3.3.5.21] "Sending a Deactivate Device Request") until any in-flight setup response arrives.
    ///
    /// [1.3.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/d7ae9bda-0dde-455b-82ef-e7858a87e46e
    /// [3.3.5.15]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/bb3d84ba-05ea-46be-9cc5-90225ffe753b
    /// [3.3.5.21]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/a0d71699-539c-4abf-b095-4ff3d0f2bf7e
    pub fn stop(&mut self) -> Option<DvcMessage> {
        let pdu = match self.state {
            DeviceState::AwaitActivation
            | DeviceState::AwaitStreams
            | DeviceState::AwaitMediaTypes
            | DeviceState::AwaitStart => {
                self.stop_requested = true;
                None
            }
            DeviceState::Streaming => {
                self.state = DeviceState::AwaitStopStreams;
                Some(DevicePdu::StopStreamsRequest)
            }
            DeviceState::AwaitStopStreams
            | DeviceState::AwaitDeactivate
            | DeviceState::AwaitFailureStopStreams
            | DeviceState::AwaitFailureDeactivate
            | DeviceState::Stopped
            | DeviceState::Failed => None,
        }?;
        Some(self.message(pdu))
    }

    /// Returns whether the client acknowledged Deactivate Device.
    ///
    /// Close the dynamic channel after this returns `true`.
    pub fn is_stopped(&self) -> bool {
        self.state == DeviceState::Stopped
    }

    /// Returns whether the device reached its terminal failure state.
    pub fn is_failed(&self) -> bool {
        self.state == DeviceState::Failed
    }

    /// Returns whether the device reached a terminal stopped or failed state.
    ///
    /// Close the dynamic channel after this returns `true`.
    pub fn is_terminated(&self) -> bool {
        matches!(self.state, DeviceState::Stopped | DeviceState::Failed)
    }

    fn deactivate(&mut self) -> DevicePdu {
        self.state = DeviceState::AwaitDeactivate;
        DevicePdu::DeactivateDeviceRequest
    }

    fn handle(&mut self, pdu: DevicePdu) -> Option<DevicePdu> {
        match (self.state, pdu) {
            (DeviceState::Stopped | DeviceState::Failed, _) => None,
            (DeviceState::AwaitActivation, DevicePdu::SuccessResponse) => {
                if self.stop_requested {
                    Some(self.deactivate())
                } else {
                    self.state = DeviceState::AwaitStreams;
                    Some(DevicePdu::StreamListRequest)
                }
            }
            (DeviceState::AwaitStreams, DevicePdu::StreamListResponse(streams)) => {
                if self.stop_requested {
                    return Some(self.deactivate());
                }
                let stream_index = streams
                    .iter()
                    .enumerate()
                    .filter(|(_, stream)| stream.frame_source_types & StreamDescription::COLOR != 0)
                    .max_by_key(|(_, stream)| stream.selected)
                    .and_then(|(index, _)| u8::try_from(index).ok());
                let Some(stream_index) = stream_index else {
                    return self.fail("no usable color stream");
                };
                self.stream_index = Some(stream_index);
                self.state = DeviceState::AwaitMediaTypes;
                Some(DevicePdu::MediaTypeListRequest(stream_index))
            }
            (DeviceState::AwaitMediaTypes, DevicePdu::MediaTypeListResponse(media_types)) => {
                if self.stop_requested {
                    return Some(self.deactivate());
                }
                let Some(media_type) = select_media_type(&media_types, self.desired, self.allow_h264) else {
                    return self.fail("no supported media type");
                };
                let Some(stream_index) = self.stream_index else {
                    return self.fail("camera stream state is missing");
                };
                self.media_type = Some(media_type);
                self.state = DeviceState::AwaitStart;
                Some(DevicePdu::StartStreamsRequest(vec![StartStreamInfo {
                    stream_index,
                    media_type,
                }]))
            }
            (DeviceState::AwaitStart, DevicePdu::SuccessResponse) => {
                if self.stop_requested {
                    self.state = DeviceState::AwaitStopStreams;
                    Some(DevicePdu::StopStreamsRequest)
                } else {
                    self.state = DeviceState::Streaming;
                    let Some(stream_index) = self.stream_index else {
                        return self.fail("camera stream state is missing");
                    };
                    Some(DevicePdu::SampleRequest(stream_index))
                }
            }
            (DeviceState::AwaitStopStreams, DevicePdu::SuccessResponse) => Some(self.deactivate()),
            (DeviceState::AwaitDeactivate, DevicePdu::SuccessResponse) => {
                self.state = DeviceState::Stopped;
                None
            }
            (DeviceState::AwaitFailureStopStreams, DevicePdu::SuccessResponse) => Some(self.failure_deactivate()),
            (DeviceState::AwaitFailureStopStreams, DevicePdu::ErrorResponse(error)) => {
                warn!(?error, "Remote camera failed to stop streams");
                Some(self.failure_deactivate())
            }
            (DeviceState::AwaitFailureDeactivate, DevicePdu::SuccessResponse) => {
                self.finish_failure();
                None
            }
            (DeviceState::AwaitFailureDeactivate, DevicePdu::ErrorResponse(error)) => {
                warn!(?error, "Remote camera failed to deactivate");
                self.finish_failure();
                None
            }
            (DeviceState::Streaming, DevicePdu::SampleResponse { stream_index, sample })
                if self.stream_index == Some(stream_index) =>
            {
                if let Some(media_type) = self.media_type
                    && media_type.validate_sample(&sample)
                {
                    (self.on_frame)(media_type, &sample);
                } else {
                    warn!(stream_index, size = sample.len(), "Dropping invalid camera sample");
                }
                Some(DevicePdu::SampleRequest(stream_index))
            }
            (DeviceState::Streaming, DevicePdu::SampleErrorResponse { stream_index, error })
                if self.stream_index == Some(stream_index) =>
            {
                warn!(stream_index, ?error, "Remote camera failed to produce a sample");
                self.fail("camera sample failed")
            }
            (_, DevicePdu::ErrorResponse(error)) => {
                warn!(?error, "Remote camera rejected a server request");
                self.fail("remote camera rejected a request")
            }
            _ => {
                warn!("Ignoring out-of-sequence camera device PDU");
                None
            }
        }
    }

    fn fail(&mut self, reason: &'static str) -> Option<DevicePdu> {
        warn!(reason, "Stopping failed camera redirection");
        match self.state {
            DeviceState::AwaitActivation | DeviceState::AwaitDeactivate | DeviceState::AwaitFailureDeactivate => {
                self.finish_failure();
                None
            }
            DeviceState::Streaming => {
                self.state = DeviceState::AwaitFailureStopStreams;
                Some(DevicePdu::StopStreamsRequest)
            }
            DeviceState::AwaitFailureStopStreams => None,
            _ => Some(self.failure_deactivate()),
        }
    }

    fn failure_deactivate(&mut self) -> DevicePdu {
        self.state = DeviceState::AwaitFailureDeactivate;
        DevicePdu::DeactivateDeviceRequest
    }

    fn finish_failure(&mut self) {
        self.state = DeviceState::Failed;
        (self.on_failure)();
    }
}

impl_as_any!(DeviceServer);

impl DvcProcessor for DeviceServer {
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(MAX_DEVICE_MESSAGE_SIZE)
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.state = DeviceState::AwaitActivation;
        self.stop_requested = false;
        self.stream_index = None;
        self.media_type = None;
        Ok(vec![self.message(DevicePdu::ActivateDeviceRequest)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let message: DeviceMessage = match decode(payload) {
            Ok(message) => message,
            Err(error) => {
                warn!(%error, "Ignoring malformed camera device PDU");
                return Ok(Vec::new());
            }
        };
        if message.version != self.version || !message.pdu.is_client_message() {
            warn!("Ignoring invalid camera device response");
            return Ok(Vec::new());
        }
        Ok(self
            .handle(message.pdu)
            .map(|pdu| vec![self.message(pdu)])
            .unwrap_or_default())
    }

    fn close(&mut self, _channel_id: u32) {
        self.state = DeviceState::Failed;
    }
}

impl DvcServerProcessor for DeviceServer {}

fn select_media_type(media_types: &[MediaType], desired: DesiredCameraFormat, allow_h264: bool) -> Option<MediaType> {
    if desired.width == 0
        || desired.height == 0
        || desired.max_width == 0
        || desired.max_height == 0
        || !desired.frame_rate.is_finite()
        || desired.frame_rate <= 0.0
    {
        return None;
    }
    return media_types
        .iter()
        .copied()
        .filter(|media_type| {
            (matches!(
                media_type.format,
                MediaFormat::Mjpg | MediaFormat::Yuy2 | MediaFormat::Nv12 | MediaFormat::I420
            ) || (allow_h264 && media_type.format == MediaFormat::H264))
                && media_type.flags & !MediaType::DECODING_REQUIRED == 0
                && media_type.width <= desired.max_width
                && media_type.height <= desired.max_height
                && media_type.validate_for_backend().is_ok()
        })
        .min_by(|left, right| {
            let left = media_type_score(left, desired);
            let right = media_type_score(right, desired);
            left.0.cmp(&right.0).then_with(|| left.1.total_cmp(&right.1))
        });

    fn media_type_score(media_type: &MediaType, desired: DesiredCameraFormat) -> (u64, f64) {
        let dimensions =
            u64::from(media_type.width.abs_diff(desired.width)) + u64::from(media_type.height.abs_diff(desired.height));
        let frame_rate = f64::from(media_type.frame_rate_numerator) / f64::from(media_type.frame_rate_denominator);
        (dimensions, (frame_rate - f64::from(desired.frame_rate)).abs())
    }
}
