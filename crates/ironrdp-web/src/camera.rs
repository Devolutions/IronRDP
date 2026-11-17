//! Camera redirection using [MS-RDPECAM: Video Capture Virtual Channel Extension].
//!
//! The browser captures and encodes, so the backend only tells the session loop what the server asked for;
//! the loop hands it to the JS callback registered with the `camera` extension, and JS pushes each frame
//! back through the `camera_sample` extension.
//!
//! [MS-RDPECAM: Video Capture Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/

use std::sync::{Arc, Mutex};

use futures_channel::mpsc;
use ironrdp_rdpecam::{
    CameraBackend, CameraRedirectionSession, DeviceChannelListener, DeviceDescriptor, EnumerationClient, ErrorCode,
    MediaFormat, MediaType, SampleDelivery, SampleSink, SampleUplink, StartStreamInfo, StreamDescription,
};
use ironrdp_svc::SvcMessage;
use tracing::{debug, warn};
use wasm_bindgen::JsValue;

use crate::session::RdpInputEvent;

/// The device channel the server opens for the camera.
const CHANNEL_NAME: &str = "RDCamera_Device_0";
/// The device name the server shows when `camera_name` is not set.
const DEFAULT_NAME: &str = "Browser camera";
const STREAM_INDEX: u8 = 0;

/// The enumeration channel and the device listener the camera registers on DRDYNVC.
pub(crate) type CameraChannels = (EnumerationClient, DeviceChannelListener<WasmCamera>);

const fn mjpeg(width: u32, height: u32, fps: u32) -> MediaType {
    MediaType {
        format: MediaFormat::Mjpg,
        width,
        height,
        frame_rate_numerator: fps,
        frame_rate_denominator: 1,
        pixel_aspect_ratio_numerator: 1,
        pixel_aspect_ratio_denominator: 1,
        flags: MediaType::DECODING_REQUIRED,
    }
}

/// The formats offered, in order of preference; the first is current until the server starts the stream.
const MEDIA_TYPES: [MediaType; 4] = [
    mjpeg(640, 480, 30),
    mjpeg(640, 480, 15),
    mjpeg(320, 240, 15),
    mjpeg(1280, 720, 15),
];

/// What the camera backend forwards to the session loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CameraMessage {
    /// The server started the stream in this format, so JS should capture in it.
    Start { width: u32, height: u32, fps: f64 },
    /// The server stopped the stream, deactivated the device or closed its channel.
    Stop,
}

#[derive(Default)]
struct Frames {
    sink: Option<SampleSink>,
    held: Option<Vec<u8>>,
}

/// Device channel PDUs for the server, encoded by the backend: the DVC processor's messages are not `Sync`,
/// which the session's input events must be, so the loop frames the bytes.
#[derive(Debug)]
pub(crate) struct CameraUplink(Vec<Vec<u8>>);

impl CameraUplink {
    pub(crate) fn into_messages(self) -> Vec<SvcMessage> {
        self.0.into_iter().map(SvcMessage::from).collect()
    }
}

/// Takes the frames JS pushes, between the session and the DVC processor.
#[derive(Clone, Default)]
pub(crate) struct CameraSink(Arc<Mutex<Frames>>);

impl CameraSink {
    /// Answers the oldest pending Sample Request with the frame and returns true, or holds the frame for the
    /// next request, in place of any older held frame, and returns false.
    pub(crate) fn push(&self, sample: impl FnOnce() -> Vec<u8>) -> bool {
        let Ok(mut guard) = self.0.lock() else {
            return false;
        };
        let frames = &mut *guard;
        let Some(sink) = frames.sink.as_ref() else {
            return false;
        };
        if sink.pending_requests(STREAM_INDEX) == 0 {
            frames.held = Some(sample());
            return false;
        }
        frames.held = None;
        sink.send_sample(STREAM_INDEX, sample()) == SampleDelivery::Sent
    }

    fn send_held(&self) {
        let Ok(mut guard) = self.0.lock() else {
            return;
        };
        let frames = &mut *guard;
        if let (Some(sink), Some(sample)) = (frames.sink.as_ref(), frames.held.take()) {
            sink.send_sample(STREAM_INDEX, sample);
        }
    }

    fn start(&self, sink: SampleSink) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = Frames {
                sink: Some(sink),
                held: None,
            };
        }
    }

    /// Drops the stream and its held frame; true when a stream was running.
    fn clear(&self) -> bool {
        let Ok(mut guard) = self.0.lock() else {
            return false;
        };
        core::mem::take(&mut *guard).sink.is_some()
    }
}

/// The camera device as the server sees it: one color stream of MJPEG frames that JS supplies.
pub(crate) struct WasmCamera {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
    sink: CameraSink,
    /// The format the server last selected, reported as the current media type until it selects another.
    selected: Option<MediaType>,
}

fn check_stream(stream_index: u8) -> Result<(), ErrorCode> {
    if stream_index == STREAM_INDEX {
        Ok(())
    } else {
        Err(ErrorCode::InvalidStreamNumber)
    }
}

impl WasmCamera {
    fn send(&self, message: CameraMessage) {
        if self.tx.unbounded_send(RdpInputEvent::Camera(message)).is_err() {
            debug!("Camera message dropped: the session loop is gone");
        }
    }

    fn stop(&mut self) {
        if self.sink.clear() {
            self.send(CameraMessage::Stop);
        }
    }
}

impl CameraBackend for WasmCamera {
    fn activate(&mut self) -> Result<(), ErrorCode> {
        Ok(())
    }

    fn deactivate(&mut self) -> Result<(), ErrorCode> {
        self.stop();
        Ok(())
    }

    fn streams(&mut self) -> Result<Vec<StreamDescription>, ErrorCode> {
        Ok(vec![StreamDescription::color(true, false)])
    }

    fn media_types(&mut self, stream_index: u8) -> Result<Vec<MediaType>, ErrorCode> {
        check_stream(stream_index)?;
        Ok(MEDIA_TYPES.to_vec())
    }

    fn current_media_type(&mut self, stream_index: u8) -> Result<MediaType, ErrorCode> {
        check_stream(stream_index)?;
        Ok(self.selected.unwrap_or(MEDIA_TYPES[0]))
    }

    fn start_streams(&mut self, streams: &[StartStreamInfo], sink: SampleSink) -> Result<(), ErrorCode> {
        let [stream] = streams else {
            return Err(ErrorCode::InvalidRequest);
        };
        check_stream(stream.stream_index)?;
        let media_type = stream.media_type;
        // A restart in another format gets a stop first, so JS never captures in the old one.
        self.stop();
        self.sink.start(sink);
        self.selected = Some(media_type);
        self.send(CameraMessage::Start {
            width: media_type.width,
            height: media_type.height,
            fps: f64::from(media_type.frame_rate_numerator) / f64::from(media_type.frame_rate_denominator),
        });
        Ok(())
    }

    fn stop_streams(&mut self) -> Result<(), ErrorCode> {
        self.stop();
        Ok(())
    }

    fn sample_requested(&mut self, _stream_index: u8) {
        self.sink.send_held();
    }

    fn shutdown(&mut self) {
        self.stop();
    }
}

/// Builds the camera's channels, announcing the device under `name` or the default when `name` is invalid.
pub(crate) fn channels(
    tx: mpsc::UnboundedSender<RdpInputEvent>,
    sink: CameraSink,
    name: Option<String>,
) -> anyhow::Result<CameraChannels> {
    let named = name.map(|name| DeviceDescriptor::new(name, CHANNEL_NAME.to_owned()));
    if let Some(Err(error)) = &named {
        warn!(%error, "Invalid camera_name; using the default");
    }
    let descriptor = match named {
        Some(Ok(descriptor)) => descriptor,
        _ => DeviceDescriptor::new(DEFAULT_NAME.to_owned(), CHANNEL_NAME.to_owned())
            .map_err(|error| anyhow::anyhow!("camera descriptor: {error}"))?,
    };
    let session = CameraRedirectionSession::new();
    let device = session
        .select_device(descriptor)
        .map_err(|error| anyhow::anyhow!("camera device: {error}"))?;
    let uplink_tx = tx.clone();
    let uplink: SampleUplink = Arc::new(move |_channel_id, messages| {
        let encoded = messages
            .iter()
            .map(SvcMessage::encode_unframed_pdu)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ironrdp_pdu::encode_err!(error))?;
        uplink_tx
            .unbounded_send(RdpInputEvent::CameraUplink(CameraUplink(encoded)))
            .map_err(|_| ironrdp_pdu::pdu_other_err!("the session loop is gone"))
    });
    let listener = device.listener(
        move || {
            Some(WasmCamera {
                tx: tx.clone(),
                sink: sink.clone(),
                selected: None,
            })
        },
        uplink,
    );
    let enumeration = session
        .enumeration_client(vec![device])
        .map_err(|error| anyhow::anyhow!("camera enumeration: {error}"))?;
    Ok((enumeration, listener))
}

/// Converts a message to the JavaScript object documented in the [camera API].
///
/// [camera API]: https://github.com/Devolutions/IronRDP/tree/master/web-client/iron-remote-desktop-rdp#camera
pub(crate) fn to_js(message: CameraMessage) -> JsValue {
    let object = js_sys::Object::new();
    let set = |key: &str, value: JsValue| {
        let _ = js_sys::Reflect::set(&object, &JsValue::from_str(key), &value);
    };
    match message {
        CameraMessage::Start { width, height, fps } => {
            set("type", JsValue::from_str("start"));
            set("width", JsValue::from(width));
            set("height", JsValue::from(height));
            set("fps", JsValue::from_f64(fps));
        }
        CameraMessage::Stop => set("type", JsValue::from_str("stop")),
    }
    object.into()
}

#[cfg(test)]
mod tests {
    use ironrdp::dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};
    use ironrdp::dvc::{DvcChannelListener as _, DvcClientProcessor, DvcProcessor as _};
    use ironrdp_core::{decode, encode_vec};
    use ironrdp_rdpecam::ProtocolVersion;
    use ironrdp_rdpecam::pdu::{DeviceMessage, DevicePdu, EnumerationPdu};

    use super::*;

    const ENUMERATION_CHANNEL_ID: u32 = 41;
    const DEVICE_CHANNEL_ID: u32 = 7;
    const JPEG: [u8; 4] = [0xFF, 0xD8, 0xFF, 0xD9];

    /// An offered MJPEG format, spelled out so the expectations do not rest on the production constructor.
    fn expected_mjpeg(width: u32, height: u32, fps: u32) -> MediaType {
        MediaType {
            format: MediaFormat::Mjpg,
            width,
            height,
            frame_rate_numerator: fps,
            frame_rate_denominator: 1,
            pixel_aspect_ratio_numerator: 1,
            pixel_aspect_ratio_denominator: 1,
            flags: MediaType::DECODING_REQUIRED,
        }
    }

    fn start_message(width: u32, height: u32, fps: f64) -> CameraMessage {
        CameraMessage::Start { width, height, fps }
    }

    struct Camera {
        device: Box<dyn DvcClientProcessor>,
        sink: CameraSink,
        rx: mpsc::UnboundedReceiver<RdpInputEvent>,
        announced: DeviceDescriptor,
    }

    /// Builds the channels, negotiates the version on the enumeration channel and opens the device channel.
    fn open(name: Option<&str>) -> Camera {
        let (tx, rx) = mpsc::unbounded();
        let sink = CameraSink::default();
        let (mut enumeration, mut listener) =
            channels(tx, sink.clone(), name.map(str::to_owned)).expect("camera channels");
        enumeration
            .start(ENUMERATION_CHANNEL_ID)
            .expect("select version request");
        let response = encode_vec(&EnumerationPdu::SelectVersionResponse(ProtocolVersion::V2)).expect("encode version");
        let added = enumeration
            .process(ENUMERATION_CHANNEL_ID, &response)
            .expect("announce the device");
        let EnumerationPdu::DeviceAdded { device: announced, .. } = decode_message(&added) else {
            panic!("expected the device to be announced");
        };
        let mut device = listener.create(DEVICE_CHANNEL_ID).expect("device channel");
        device.start(DEVICE_CHANNEL_ID).expect("device channel start");
        Camera {
            device,
            sink,
            rx,
            announced,
        }
    }

    /// An activated camera whose stream the server started in `media_type`, its start message consumed.
    fn started(media_type: MediaType) -> Camera {
        let mut camera = open(None);
        camera.activate();
        camera.start(media_type);
        camera.next_message();
        camera
    }

    impl Camera {
        fn exchange(&mut self, request: DevicePdu) -> Option<DevicePdu> {
            let encoded = encode_vec(&DeviceMessage::new(ProtocolVersion::V2, request)).expect("encode request");
            let response = self
                .device
                .process(DEVICE_CHANNEL_ID, &encoded)
                .expect("process request");
            (!response.is_empty()).then(|| decode_message::<DeviceMessage>(&response).pdu)
        }

        fn activate(&mut self) {
            assert_eq!(
                self.exchange(DevicePdu::ActivateDeviceRequest),
                Some(DevicePdu::SuccessResponse)
            );
        }

        fn start(&mut self, media_type: MediaType) {
            let start = StartStreamInfo {
                stream_index: STREAM_INDEX,
                media_type,
            };
            assert_eq!(
                self.exchange(DevicePdu::StartStreamsRequest(vec![start])),
                Some(DevicePdu::SuccessResponse)
            );
        }

        fn stop(&mut self) {
            assert_eq!(
                self.exchange(DevicePdu::StopStreamsRequest),
                Some(DevicePdu::SuccessResponse)
            );
        }

        fn request_sample(&mut self) {
            assert_eq!(self.exchange(DevicePdu::SampleRequest(STREAM_INDEX)), None);
        }

        fn current_media_type(&mut self) -> Option<DevicePdu> {
            self.exchange(DevicePdu::CurrentMediaTypeRequest(STREAM_INDEX))
        }

        fn next_message(&mut self) -> CameraMessage {
            match self.rx.try_recv() {
                Ok(RdpInputEvent::Camera(message)) => message,
                other => panic!("expected a camera message, got {other:?}"),
            }
        }

        /// The device PDUs the backend sent the server since the last call.
        fn uplinked(&mut self) -> Vec<DevicePdu> {
            let mut sent = Vec::new();
            while let Ok(event) = self.rx.try_recv() {
                let RdpInputEvent::CameraUplink(uplink) = event else {
                    panic!("expected an uplink, got {event:?}");
                };
                for message in uplink.into_messages() {
                    let encoded = message.encode_unframed_pdu().expect("encode DVC message");
                    let DrdynvcClientPdu::Data(data) =
                        decode::<DrdynvcClientPdu>(&encoded).expect("decode DVC message")
                    else {
                        panic!("expected a DVC data PDU");
                    };
                    assert_eq!(data.channel_id(), DEVICE_CHANNEL_ID);
                    let DrdynvcDataPdu::Data(data) = data else {
                        panic!("expected an unfragmented DVC data PDU");
                    };
                    sent.push(decode::<DeviceMessage>(data.data()).expect("decode device message").pdu);
                }
            }
            sent
        }

        fn assert_quiet(&mut self) {
            let event = self.rx.try_recv();
            assert!(
                matches!(event, Err(mpsc::TryRecvError::Empty)),
                "expected no event, got {event:?}"
            );
        }
    }

    fn decode_message<T>(messages: &[ironrdp::dvc::DvcMessage]) -> T
    where
        T: for<'de> ironrdp_core::Decode<'de>,
    {
        assert_eq!(messages.len(), 1);
        let encoded = encode_vec(&*messages[0]).expect("encode DVC message");
        decode(&encoded).expect("decode DVC message")
    }

    fn sample_response(sample: &[u8]) -> DevicePdu {
        DevicePdu::SampleResponse {
            stream_index: STREAM_INDEX,
            sample: sample.to_vec(),
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn the_device_is_announced_under_the_configured_name() {
        // oracle: spec MS-RDPECAM §2.2.2.3 Device Added Notification carries the name shown to the user.
        // oracle: mutant dropping the configured name.
        assert_eq!(open(Some("Desk camera")).announced.display_name, "Desk camera");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn the_device_is_announced_under_the_default_name_when_none_is_given() {
        // oracle: mutant changing or dropping the default name.
        assert_eq!(open(None).announced.display_name, "Browser camera");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn an_invalid_name_falls_back_to_the_default() {
        // oracle: reference ironrdp-rdpecam rejects a device name with a NUL; the fallback is this module's.
        // oracle: mutant failing the channels, or announcing the invalid name.
        assert_eq!(open(Some("bad\0name")).announced.display_name, "Browser camera");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn offers_one_color_stream() {
        // oracle: spec MS-RDPECAM §2.2.3.6 Stream List Response.
        // oracle: mutant offering another stream count or a shareable stream.
        let mut camera = open(None);
        camera.activate();
        assert_eq!(
            camera.exchange(DevicePdu::StreamListRequest),
            Some(DevicePdu::StreamListResponse(vec![StreamDescription::color(
                true, false
            )]))
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn offers_the_mjpeg_formats_in_order() {
        // oracle: spec MS-RDPECAM §2.2.3.8 Media Type List Response lists the formats in the client's order of preference.
        // oracle: mutant changing the offered formats or their order.
        let mut camera = open(None);
        camera.activate();
        assert_eq!(
            camera.exchange(DevicePdu::MediaTypeListRequest(STREAM_INDEX)),
            Some(DevicePdu::MediaTypeListResponse(vec![
                expected_mjpeg(640, 480, 30),
                expected_mjpeg(640, 480, 15),
                expected_mjpeg(320, 240, 15),
                expected_mjpeg(1280, 720, 15),
            ]))
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn another_stream_index_is_an_invalid_stream_number() {
        // oracle: spec MS-RDPECAM §2.2.3.2 Error Response defines InvalidStreamNumber.
        // oracle: mutant answering a second stream.
        let mut camera = open(None);
        camera.activate();
        assert_eq!(
            camera.exchange(DevicePdu::MediaTypeListRequest(1)),
            Some(DevicePdu::ErrorResponse(ErrorCode::InvalidStreamNumber))
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn the_first_format_is_current_before_a_start() {
        // oracle: spec MS-RDPECAM §2.2.3.10 Current Media Type Response carries one media type; which one is this module's rule.
        // oracle: mutant reporting another format than the first offered before the server selects one (the README's rule).
        let mut camera = open(None);
        camera.activate();
        assert_eq!(
            camera.current_media_type(),
            Some(DevicePdu::CurrentMediaTypeResponse(expected_mjpeg(640, 480, 30)))
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn the_selected_format_stays_current_after_a_stop() {
        // oracle: spec MS-RDPECAM §2.2.3.10 Current Media Type Response reports the format the server selected.
        // oracle: mutant resetting the current format when the stream stops.
        let mut camera = started(expected_mjpeg(320, 240, 15));
        camera.stop();
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        assert_eq!(
            camera.current_media_type(),
            Some(DevicePdu::CurrentMediaTypeResponse(expected_mjpeg(320, 240, 15)))
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_start_names_the_format() {
        // oracle: spec MS-RDPECAM §2.2.3.11 Start Streams Request carries the selected media type.
        // oracle: mutant dropping the start message or its fields.
        let mut camera = open(None);
        camera.activate();
        camera.start(expected_mjpeg(640, 480, 30));
        assert_eq!(camera.next_message(), start_message(640, 480, 30.0));
        camera.assert_quiet();
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_second_stop_is_quiet() {
        // oracle: spec MS-RDPECAM §2.2.3.12 Stop Streams Request.
        // oracle: mutant repeating the stop message.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        camera.stop();
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        camera.stop();
        camera.assert_quiet();
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn deactivation_stops_the_capture() {
        // oracle: spec MS-RDPECAM §2.2.3.4 Deactivate Device Request ends the device's use.
        // oracle: mutant ignoring a deactivation while a stream runs.
        let mut camera = started(expected_mjpeg(640, 480, 15));
        assert_eq!(
            camera.exchange(DevicePdu::DeactivateDeviceRequest),
            Some(DevicePdu::SuccessResponse)
        );
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        camera.assert_quiet();
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_restart_in_another_format_stops_first() {
        // oracle: mutant starting the new format without stopping the old one.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        camera.start(expected_mjpeg(320, 240, 15));
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        assert_eq!(camera.next_message(), start_message(320, 240, 15.0));
        // oracle: spec MS-RDPECAM §2.2.3.10 Current Media Type Response reports the format of the running stream.
        assert_eq!(
            camera.current_media_type(),
            Some(DevicePdu::CurrentMediaTypeResponse(expected_mjpeg(320, 240, 15)))
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_frame_answers_a_pending_request_at_once() {
        // oracle: spec MS-RDPECAM §2.2.3.13 Sample Request and §2.2.3.14 Sample Response.
        // oracle: mutant holding a frame a request is already waiting for.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        camera.request_sample();
        assert!(camera.sink.push(|| JPEG.to_vec()));
        assert_eq!(camera.uplinked(), [sample_response(&JPEG)]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_frame_answers_one_pending_request_and_leaves_the_rest() {
        // oracle: spec MS-RDPECAM §2.2.3.13 Sample Request and §2.2.3.14 Sample Response: each response answers one request.
        // oracle: mutant answering every pending request with one frame, or holding a frame while a request is still pending.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        camera.request_sample();
        camera.request_sample();
        assert!(camera.sink.push(|| JPEG.to_vec()));
        assert_eq!(camera.uplinked(), [sample_response(&JPEG)]);
        let next = [JPEG.as_slice(), &[1]].concat();
        assert!(camera.sink.push(|| next.clone()));
        assert_eq!(camera.uplinked(), [sample_response(&next)]);
        assert!(!camera.sink.push(|| JPEG.to_vec()));
        camera.assert_quiet();
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn the_newest_frame_waits_for_the_next_request() {
        // oracle: spec MS-RDPECAM §2.2.3.13 Sample Request and §2.2.3.14 Sample Response: a response answers a request.
        // oracle: mutant sending a frame nobody asked for, or keeping an older frame over a newer one.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        let older = [JPEG.as_slice(), &[1]].concat();
        let newer = [JPEG.as_slice(), &[2]].concat();
        assert!(!camera.sink.push(|| older));
        assert!(!camera.sink.push(|| newer.clone()));
        camera.assert_quiet();
        camera.request_sample();
        assert_eq!(camera.uplinked(), [sample_response(&newer)]);
        camera.request_sample();
        camera.assert_quiet();
        assert!(camera.sink.push(|| JPEG.to_vec()));
        assert_eq!(camera.uplinked(), [sample_response(&JPEG)]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_held_frame_does_not_outlive_its_stream() {
        // oracle: spec MS-RDPECAM §2.2.3.12 Stop Streams Request ends the stream the frame was captured for.
        // oracle: mutant answering a new stream's first request with a frame of the old one, or sending after a stop.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        assert!(!camera.sink.push(|| JPEG.to_vec()));
        camera.stop();
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        assert!(!camera.sink.push(|| JPEG.to_vec()));
        camera.start(expected_mjpeg(320, 240, 15));
        camera.next_message();
        camera.request_sample();
        camera.assert_quiet();
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn closing_the_channel_stops_the_capture() {
        // oracle: reference ironrdp-rdpecam's DeviceClient releases the device when the server closes its channel.
        // oracle: mutant leaving the page capturing after the channel is gone, or accepting frames afterwards.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        camera.device.close(DEVICE_CHANNEL_ID);
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        assert!(!camera.sink.push(|| JPEG.to_vec()));
        camera.assert_quiet();
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_frame_that_is_not_a_jpeg_is_refused() {
        // oracle: spec MS-RDPECAM §2.2.3.15 Sample Error Response answers a request the sample cannot.
        // oracle: reference ironrdp-rdpecam's SampleSink refuses an MJPEG sample without the JPEG SOI marker.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        camera.request_sample();
        assert!(!camera.sink.push(|| vec![1, 2, 3]));
        assert_eq!(
            camera.uplinked(),
            [DevicePdu::SampleErrorResponse {
                stream_index: STREAM_INDEX,
                error: ErrorCode::InvalidMediaType,
            }]
        );
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn field(message: &JsValue, name: &str) -> JsValue {
        js_sys::Reflect::get(message, &JsValue::from_str(name)).expect("read camera message field")
    }

    fn keys(message: &JsValue) -> Vec<String> {
        let mut keys: Vec<_> = js_sys::Object::keys(message.unchecked_ref::<js_sys::Object>())
            .iter()
            .map(|key| key.as_string().expect("string key"))
            .collect();
        keys.sort();
        keys
    }

    #[wasm_bindgen_test]
    fn start_callback_contract() {
        // oracle: mutant renaming start fields or changing their values.
        let message = to_js(CameraMessage::Start {
            width: 640,
            height: 480,
            fps: 30.0,
        });
        assert_eq!(keys(&message), ["fps", "height", "type", "width"]);
        assert_eq!(field(&message, "type"), "start");
        assert_eq!(field(&message, "width"), 640);
        assert_eq!(field(&message, "height"), 480);
        assert_eq!(field(&message, "fps"), 30.0);
    }

    #[wasm_bindgen_test]
    fn stop_callback_contract() {
        // oracle: mutant changing the stop discriminator or adding unrelated fields.
        let message = to_js(CameraMessage::Stop);
        assert_eq!(keys(&message), ["type"]);
        assert_eq!(field(&message, "type"), "stop");
    }
}
