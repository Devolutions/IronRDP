//! Camera redirection using [MS-RDPECAM: Video Capture Virtual Channel Extension].
//!
//! The browser captures and encodes, so the backend only tells the session loop what the server asked for;
//! the loop hands it to the JS callback registered with the `camera` extension, and JS pushes each frame
//! back, as JPEG or H.264, through the `camera_sample` extension.
//!
//! [MS-RDPECAM: Video Capture Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/

use std::collections::VecDeque;
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

/// Every H.264 picture but a key frame depends on the ones before it, so held H.264 frames queue in order, up to this many.
const ORDERED_HELD_MAX: usize = 30;

/// The sizes offered, in order of preference: width, height and frames per second.
const SIZES: [(u32, u32, u32); 4] = [(640, 480, 30), (640, 480, 15), (320, 240, 15), (1280, 720, 15)];

const fn media_type(format: MediaFormat, width: u32, height: u32, fps: u32) -> MediaType {
    MediaType {
        format,
        width,
        height,
        frame_rate_numerator: fps,
        frame_rate_denominator: 1,
        pixel_aspect_ratio_numerator: 1,
        pixel_aspect_ratio_denominator: 1,
        flags: MediaType::DECODING_REQUIRED,
    }
}

/// The formats offered, in order of preference: H.264 ahead of MJPEG when `h264` is set, MJPEG alone otherwise.
fn formats(h264: bool) -> &'static [MediaFormat] {
    if h264 {
        &[MediaFormat::H264, MediaFormat::Mjpg]
    } else {
        &[MediaFormat::Mjpg]
    }
}

/// The media types offered, in order of preference: every size of each format in [`formats`].
/// The first is current until the server selects one.
fn media_types(h264: bool) -> Vec<MediaType> {
    formats(h264)
        .iter()
        .flat_map(|&format| {
            SIZES
                .iter()
                .map(move |&(width, height, fps)| media_type(format, width, height, fps))
        })
        .collect()
}

/// The first of [`media_types`], from the same tables, built on its own: the fallback current media type needs
/// no other.
fn first_media_type(h264: bool) -> MediaType {
    let (width, height, fps) = SIZES[0];
    media_type(formats(h264)[0], width, height, fps)
}

/// What the camera backend forwards to the session loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CameraMessage {
    /// The server started the stream in this format, so JS should capture in it, encoding H.264 or JPEG.
    Start {
        width: u32,
        height: u32,
        fps: f64,
        h264: bool,
    },
    /// The server stopped the stream, deactivated the device or closed its channel.
    Stop,
}

#[derive(Default)]
struct Frames {
    sink: Option<SampleSink>,
    /// INVARIANT: held frames were captured for the stream in `sink`; `start` and `clear` drop them with it.
    held: VecDeque<Vec<u8>>,
    /// Set for an H.264 stream, whose held frames go out in order and never replace each other: how to make JS
    /// capture afresh when the queue overflows or the server refuses a held frame. `None` for MJPEG, where a held
    /// frame replaces the one before.
    restart: Option<Restart>,
}

/// The stop and start messages that make JS capture afresh, so that its next frame is a key frame.
struct Restart {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
    start: CameraMessage,
}

impl Restart {
    fn send(&self) {
        send(&self.tx, CameraMessage::Stop);
        send(&self.tx, self.start);
    }
}

fn send(tx: &mpsc::UnboundedSender<RdpInputEvent>, message: CameraMessage) {
    if tx.unbounded_send(RdpInputEvent::Camera(message)).is_err() {
        debug!("Camera message dropped: the session loop is gone");
    }
}

/// Answers as many pending Sample Requests as there are held frames, oldest first.
/// Returns true when a frame went out and none is left held.
fn flush(sink: &SampleSink, held: &mut VecDeque<Vec<u8>>, restart: Option<&Restart>) -> bool {
    let mut sent = false;
    while 0 < sink.pending_requests(STREAM_INDEX) {
        let Some(sample) = held.front() else {
            break;
        };
        // The sink takes the sample; the copy lets a failed delivery keep the frame for the next try.
        match sink.send_sample(STREAM_INDEX, sample.clone()) {
            SampleDelivery::Sent => {
                held.pop_front();
                sent = true;
            }
            SampleDelivery::Rejected => {
                // The server got a Sample Error Response in its place. The H.264 frames behind it would decode
                // against a picture the server never gets, so they go and the capture restarts.
                held.pop_front();
                if let Some(restart) = restart {
                    warn!(
                        "The server refused a held H.264 frame; dropping the ones behind it and restarting the capture"
                    );
                    held.clear();
                    restart.send();
                }
                break;
            }
            // A delivery that fails leaves its request outstanding and the frame held for the next try.
            _ => break,
        }
    }
    sent && held.is_empty()
}

/// Device channel PDUs for the server, encoded by the backend.
///
/// The DVC processor's messages are `Send` but not `Sync`, and the session's input events must be `Sync`: the
/// existing `unbounded_send(..).context(..)` calls need `TrySendError<RdpInputEvent>` to satisfy `anyhow`'s
/// `Send + Sync` error bound. So the backend frames the bytes and the loop wraps them again.
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
    /// Sends the frame if the server has asked for one, and holds it otherwise: an MJPEG frame replaces any
    /// held frame, an H.264 frame queues behind them. Returns true when a frame went out and none is left held.
    pub(crate) fn push(&self, sample: Vec<u8>) -> bool {
        let Ok(mut guard) = self.0.lock() else {
            return false;
        };
        let Frames { sink, held, restart } = &mut *guard;
        let Some(sink) = sink.as_ref() else {
            return false;
        };
        match restart {
            None => held.clear(),
            Some(restart) if ORDERED_HELD_MAX <= held.len() => {
                // This frame and the ones behind it decode against pictures the server never gets, so the capture
                // restarts: JS gets a stop and a start, and its next frame is a key frame.
                warn!(
                    "Camera frames are not being requested; dropping the held H.264 frames and restarting the capture"
                );
                held.clear();
                restart.send();
                return false;
            }
            Some(_) => {}
        }
        held.push_back(sample);
        flush(sink, held, restart.as_ref())
    }

    /// How many frames JS should encode now: the Sample Requests no held frame answers, plus one held ready
    /// for the next request, which then goes out without waiting for an encode.
    pub(crate) fn wanted(&self) -> u32 {
        let Ok(guard) = self.0.lock() else {
            return 0;
        };
        guard.sink.as_ref().map_or(0, |sink| {
            let held = u32::try_from(guard.held.len()).unwrap_or(u32::MAX);
            sink.pending_requests(STREAM_INDEX)
                .saturating_add(1)
                .saturating_sub(held)
        })
    }

    fn send_held(&self) {
        let Ok(mut guard) = self.0.lock() else {
            return;
        };
        let Frames { sink, held, restart } = &mut *guard;
        if let Some(sink) = sink.as_ref() {
            flush(sink, held, restart.as_ref());
        }
    }

    /// Takes the stream's sink, with the restart that marks an H.264 stream.
    fn start(&self, sink: SampleSink, restart: Option<Restart>) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = Frames {
                sink: Some(sink),
                held: VecDeque::new(),
                restart,
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

/// The camera device as the server sees it: one color stream of H.264 or MJPEG frames that JS supplies.
pub(crate) struct WasmCamera {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
    sink: CameraSink,
    h264: bool,
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
        send(&self.tx, message);
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
        Ok(media_types(self.h264))
    }

    fn current_media_type(&mut self, stream_index: u8) -> Result<MediaType, ErrorCode> {
        check_stream(stream_index)?;
        Ok(self.selected.unwrap_or_else(|| first_media_type(self.h264)))
    }

    fn start_streams(&mut self, streams: &[StartStreamInfo], sink: SampleSink) -> Result<(), ErrorCode> {
        let [stream] = streams else {
            return Err(ErrorCode::InvalidRequest);
        };
        check_stream(stream.stream_index)?;
        let media_type = stream.media_type;
        let h264 = media_type.format == MediaFormat::H264;
        // A restart in another format gets a stop first, so JS never captures in the old one.
        self.stop();
        let start = CameraMessage::Start {
            width: media_type.width,
            height: media_type.height,
            fps: f64::from(media_type.frame_rate_numerator) / f64::from(media_type.frame_rate_denominator),
            h264,
        };
        let restart = Restart {
            tx: self.tx.clone(),
            start,
        };
        self.sink.start(sink, h264.then_some(restart));
        self.selected = Some(media_type);
        self.send(start);
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
    h264: bool,
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
                h264,
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
        CameraMessage::Start {
            width,
            height,
            fps,
            h264,
        } => {
            set("type", JsValue::from_str("start"));
            set("width", JsValue::from(width));
            set("height", JsValue::from(height));
            set("fps", JsValue::from_f64(fps));
            set("format", JsValue::from_str(if h264 { "h264" } else { "mjpeg" }));
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

    /// An offered H.264 format, spelled out like `expected_mjpeg`.
    fn expected_h264(width: u32, height: u32, fps: u32) -> MediaType {
        MediaType {
            format: MediaFormat::H264,
            ..expected_mjpeg(width, height, fps)
        }
    }

    /// An Annex B start code, which is all the crate checks of an H.264 sample, and a tag to tell frames apart.
    fn h264_frame(tag: u8) -> Vec<u8> {
        vec![0, 0, 0, 1, tag]
    }

    fn start_message(width: u32, height: u32, fps: f64, h264: bool) -> CameraMessage {
        CameraMessage::Start {
            width,
            height,
            fps,
            h264,
        }
    }

    struct Camera {
        device: Box<dyn DvcClientProcessor>,
        sink: CameraSink,
        rx: mpsc::UnboundedReceiver<RdpInputEvent>,
        announced: DeviceDescriptor,
    }

    /// Builds the channels, negotiates the version on the enumeration channel and opens the device channel.
    fn open(name: Option<&str>, with_h264: bool) -> Camera {
        let (tx, rx) = mpsc::unbounded();
        let sink = CameraSink::default();
        let (mut enumeration, mut listener) =
            channels(tx, sink.clone(), name.map(str::to_owned), with_h264).expect("camera channels");
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

    /// An activated camera, offering H.264 when `h264` is set, whose stream the server started in `media_type`,
    /// its start message consumed.
    fn started_with(media_type: MediaType, h264: bool) -> Camera {
        let mut camera = open(None, h264);
        camera.activate();
        camera.start(media_type);
        camera.next_message();
        camera
    }

    /// An activated camera offering what `media_type`'s format needs, started in it.
    fn started(media_type: MediaType) -> Camera {
        started_with(media_type, media_type.format == MediaFormat::H264)
    }

    /// An activated camera offering H.264, started in `media_type`, H.264 or MJPEG.
    fn started_with_h264(media_type: MediaType) -> Camera {
        started_with(media_type, true)
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
                sent.extend(decode_uplink(event));
            }
            sent
        }

        /// The device PDUs of the next event, which must be an uplink.
        fn next_uplink(&mut self) -> Vec<DevicePdu> {
            let event = self.rx.try_recv().expect("an uplink event");
            decode_uplink(event)
        }

        fn assert_quiet(&mut self) {
            let event = self.rx.try_recv();
            assert!(
                matches!(event, Err(mpsc::TryRecvError::Empty)),
                "expected no event, got {event:?}"
            );
        }
    }

    /// The device PDUs an uplink event carries.
    fn decode_uplink(event: RdpInputEvent) -> Vec<DevicePdu> {
        let RdpInputEvent::CameraUplink(uplink) = event else {
            panic!("expected an uplink, got {event:?}");
        };
        uplink
            .into_messages()
            .into_iter()
            .map(|message| {
                let encoded = message.encode_unframed_pdu().expect("encode DVC message");
                let DrdynvcClientPdu::Data(data) = decode::<DrdynvcClientPdu>(&encoded).expect("decode DVC message")
                else {
                    panic!("expected a DVC data PDU");
                };
                assert_eq!(data.channel_id(), DEVICE_CHANNEL_ID);
                let DrdynvcDataPdu::Data(data) = data else {
                    panic!("expected an unfragmented DVC data PDU");
                };
                decode::<DeviceMessage>(data.data()).expect("decode device message").pdu
            })
            .collect()
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
        assert_eq!(open(Some("Desk camera"), false).announced.display_name, "Desk camera");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn the_device_is_announced_under_the_default_name_when_none_is_given() {
        // oracle: mutant changing or dropping the default name.
        assert_eq!(open(None, false).announced.display_name, "Browser camera");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn an_invalid_name_falls_back_to_the_default() {
        // oracle: reference ironrdp-rdpecam rejects a device name with a NUL; the fallback is this module's.
        // oracle: mutant failing the channels, or announcing the invalid name.
        assert_eq!(open(Some("bad\0name"), false).announced.display_name, "Browser camera");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn offers_one_color_stream() {
        // oracle: spec MS-RDPECAM §2.2.3.6 Stream List Response.
        // oracle: mutant offering another stream count or a shareable stream.
        let mut camera = open(None, false);
        camera.activate();
        assert_eq!(
            camera.exchange(DevicePdu::StreamListRequest),
            Some(DevicePdu::StreamListResponse(vec![StreamDescription {
                frame_source_types: StreamDescription::COLOR,
                selected: true,
                can_be_shared: false,
            }]))
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn offers_the_mjpeg_formats_in_order() {
        // oracle: spec MS-RDPECAM §2.2.3.8 Media Type List Response lists the formats in the client's order of preference.
        // oracle: mutant changing the offered formats or their order.
        let mut camera = open(None, false);
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
        let mut camera = open(None, false);
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
        let mut camera = open(None, false);
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
        let mut camera = open(None, false);
        camera.activate();
        camera.start(expected_mjpeg(640, 480, 30));
        assert_eq!(camera.next_message(), start_message(640, 480, 30.0, false));
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
        assert_eq!(camera.next_message(), start_message(320, 240, 15.0, false));
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
        assert!(camera.sink.push(JPEG.to_vec()));
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
        assert!(camera.sink.push(JPEG.to_vec()));
        assert_eq!(camera.uplinked(), [sample_response(&JPEG)]);
        let next = [JPEG.as_slice(), &[1]].concat();
        assert!(camera.sink.push(next.clone()));
        assert_eq!(camera.uplinked(), [sample_response(&next)]);
        assert!(!camera.sink.push(JPEG.to_vec()));
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
        assert!(!camera.sink.push(older));
        assert!(!camera.sink.push(newer.clone()));
        camera.assert_quiet();
        camera.request_sample();
        assert_eq!(camera.uplinked(), [sample_response(&newer)]);
        camera.request_sample();
        camera.assert_quiet();
        assert!(camera.sink.push(JPEG.to_vec()));
        assert_eq!(camera.uplinked(), [sample_response(&JPEG)]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_held_frame_does_not_outlive_its_stream() {
        // oracle: spec MS-RDPECAM §2.2.3.12 Stop Streams Request ends the stream the frame was captured for.
        // oracle: mutant answering a new stream's first request with a frame of the old one, or sending after a stop.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        assert!(!camera.sink.push(JPEG.to_vec()));
        camera.stop();
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        assert!(!camera.sink.push(JPEG.to_vec()));
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
        assert!(!camera.sink.push(JPEG.to_vec()));
        camera.assert_quiet();
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_frame_that_is_not_a_jpeg_is_refused() {
        // oracle: spec MS-RDPECAM §2.2.3.15 Sample Error Response answers a request the sample cannot.
        // oracle: reference ironrdp-rdpecam's SampleSink refuses an MJPEG sample without the JPEG SOI marker.
        let mut camera = started(expected_mjpeg(640, 480, 30));
        camera.request_sample();
        assert!(!camera.sink.push(vec![1, 2, 3]));
        assert_eq!(
            camera.uplinked(),
            [DevicePdu::SampleErrorResponse {
                stream_index: STREAM_INDEX,
                error: ErrorCode::InvalidMediaType,
            }]
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn h264_is_offered_ahead_of_mjpeg_when_enabled() {
        // oracle: spec MS-RDPECAM §2.2.3.8 Media Type List Response lists the formats in the client's order of preference.
        // oracle: mutant offering MJPEG only, or putting H.264 after MJPEG.
        let mut camera = open(None, true);
        camera.activate();
        assert_eq!(
            camera.exchange(DevicePdu::MediaTypeListRequest(STREAM_INDEX)),
            Some(DevicePdu::MediaTypeListResponse(vec![
                expected_h264(640, 480, 30),
                expected_h264(640, 480, 15),
                expected_h264(320, 240, 15),
                expected_h264(1280, 720, 15),
                expected_mjpeg(640, 480, 30),
                expected_mjpeg(640, 480, 15),
                expected_mjpeg(320, 240, 15),
                expected_mjpeg(1280, 720, 15),
            ]))
        );
        // oracle: spec MS-RDPECAM §2.2.3.10 Current Media Type Response carries one media type; which one is this module's rule.
        // oracle: mutant reporting another format than the first offered before the server selects one (the README's rule).
        assert_eq!(
            camera.current_media_type(),
            Some(DevicePdu::CurrentMediaTypeResponse(expected_h264(640, 480, 30)))
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_start_names_h264_or_mjpeg() {
        // oracle: spec MS-RDPECAM §2.2.3.11 Start Streams Request carries the selected media type.
        // oracle: mutant naming the wrong format in a start.
        let mut camera = open(None, true);
        camera.activate();
        camera.start(expected_h264(1280, 720, 15));
        assert_eq!(camera.next_message(), start_message(1280, 720, 15.0, true));
        camera.start(expected_mjpeg(320, 240, 15));
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        assert_eq!(camera.next_message(), start_message(320, 240, 15.0, false));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn held_h264_frames_go_out_in_order() {
        // oracle: mutant replacing a held H.264 frame or reordering the queue (this module's rule: an H.264 picture other than an IDR is decoded from the pictures before it, so none may be dropped or reordered).
        let mut camera = started_with_h264(expected_h264(640, 480, 30));
        assert!(!camera.sink.push(h264_frame(1)));
        assert!(!camera.sink.push(h264_frame(2)));
        camera.assert_quiet();
        camera.request_sample();
        assert_eq!(camera.uplinked(), [sample_response(&h264_frame(1))]);
        camera.request_sample();
        assert_eq!(camera.uplinked(), [sample_response(&h264_frame(2))]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_refused_held_h264_frame_restarts_the_capture() {
        // oracle: reference ironrdp-rdpecam refuses an H.264 sample without an Annex B start code when it is sent.
        // oracle: mutant keeping the frames behind a refused one, or going on without a restart.
        let mut camera = started_with_h264(expected_h264(640, 480, 30));
        assert!(!camera.sink.push(vec![1, 2, 3]));
        assert!(!camera.sink.push(h264_frame(2)));
        camera.request_sample();
        assert_eq!(
            camera.next_uplink(),
            [DevicePdu::SampleErrorResponse {
                stream_index: STREAM_INDEX,
                error: ErrorCode::InvalidMediaType,
            }]
        );
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        assert_eq!(camera.next_message(), start_message(640, 480, 30.0, true));
        camera.request_sample();
        camera.assert_quiet();
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn mjpeg_frames_still_replace_each_other_with_h264_offered() {
        // oracle: mutant queueing MJPEG frames because H.264 is offered.
        let mut camera = started_with_h264(expected_mjpeg(640, 480, 30));
        let older = [JPEG.as_slice(), &[1]].concat();
        let newer = [JPEG.as_slice(), &[2]].concat();
        assert!(!camera.sink.push(older));
        assert!(!camera.sink.push(newer.clone()));
        camera.request_sample();
        assert_eq!(camera.uplinked(), [sample_response(&newer)]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn wanted_is_the_unanswered_requests_plus_one_minus_the_held_frames() {
        // oracle: spec MS-RDPECAM §2.2.3.13 Sample Request: the server asks for one sample at a time.
        // oracle: mutant dropping the spare frame or ignoring the held ones (the plus-one is this module's rule, in the README).
        let mut camera = open(None, true);
        assert_eq!(camera.sink.wanted(), 0);
        camera.activate();
        camera.start(expected_h264(640, 480, 30));
        camera.next_message();
        assert_eq!(camera.sink.wanted(), 1);
        camera.request_sample();
        camera.request_sample();
        assert_eq!(camera.sink.wanted(), 3);
        assert!(camera.sink.push(h264_frame(1)));
        assert_eq!(camera.sink.wanted(), 2);
        assert!(camera.sink.push(h264_frame(2)));
        assert_eq!(camera.sink.wanted(), 1);
        assert!(!camera.sink.push(h264_frame(3)));
        assert_eq!(camera.sink.wanted(), 0);
        camera.request_sample();
        assert_eq!(camera.sink.wanted(), 1);
        assert_eq!(
            camera.uplinked(),
            [
                sample_response(&h264_frame(1)),
                sample_response(&h264_frame(2)),
                sample_response(&h264_frame(3)),
            ]
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn an_overflowing_h264_queue_restarts_the_capture() {
        // oracle: mutant letting the queue grow without bound, keeping the stream going after the drop, or keeping
        // the frame pushed into a full queue.
        let mut camera = started_with_h264(expected_h264(640, 480, 30));
        for tag in 1..=30 {
            assert!(!camera.sink.push(h264_frame(tag)));
        }
        assert_eq!(camera.sink.wanted(), 0);
        camera.assert_quiet();
        assert!(!camera.sink.push(h264_frame(31)));
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        assert_eq!(camera.next_message(), start_message(640, 480, 30.0, true));
        camera.request_sample();
        camera.assert_quiet();
        assert!(camera.sink.push(h264_frame(32)));
        assert_eq!(camera.uplinked(), [sample_response(&h264_frame(32))]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_restart_drops_the_held_h264_queue() {
        // oracle: mutant carrying the held frames of the old stream into the new one.
        let mut camera = started_with_h264(expected_h264(640, 480, 30));
        assert!(!camera.sink.push(h264_frame(1)));
        assert!(!camera.sink.push(h264_frame(2)));
        camera.start(expected_h264(320, 240, 15));
        assert_eq!(camera.next_message(), CameraMessage::Stop);
        assert_eq!(camera.next_message(), start_message(320, 240, 15.0, true));
        camera.request_sample();
        camera.assert_quiet();
        assert!(camera.sink.push(h264_frame(3)));
        assert_eq!(camera.uplinked(), [sample_response(&h264_frame(3))]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_failed_delivery_keeps_every_held_frame() {
        // oracle: reference ironrdp-rdpecam leaves the request outstanding when a delivery fails.
        // oracle: mutant dropping the head frame, or the whole queue, on a failure.
        let mut camera = started_with_h264(expected_h264(640, 480, 30));
        assert!(!camera.sink.push(h264_frame(1)));
        assert!(!camera.sink.push(h264_frame(2)));
        // The session loop is gone: every delivery fails.
        camera.rx = mpsc::unbounded().1;
        camera.request_sample();
        assert_eq!(camera.sink.wanted(), 0);
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

    fn assert_start_message(h264: bool, format: &str) {
        let message = to_js(CameraMessage::Start {
            width: 640,
            height: 480,
            fps: 30.0,
            h264,
        });
        assert_eq!(keys(&message), ["format", "fps", "height", "type", "width"]);
        assert_eq!(field(&message, "type"), "start");
        assert_eq!(field(&message, "width"), 640);
        assert_eq!(field(&message, "height"), 480);
        assert_eq!(field(&message, "fps"), 30.0);
        assert_eq!(field(&message, "format"), format);
    }

    #[wasm_bindgen_test]
    fn start_callback_contract_for_mjpeg() {
        // oracle: mutant renaming start fields, changing their values or naming the wrong format.
        assert_start_message(false, "mjpeg");
    }

    #[wasm_bindgen_test]
    fn start_callback_contract_for_h264() {
        // oracle: mutant naming the wrong format for an H.264 start.
        assert_start_message(true, "h264");
    }

    #[wasm_bindgen_test]
    fn stop_callback_contract() {
        // oracle: mutant changing the stop discriminator or adding unrelated fields.
        let message = to_js(CameraMessage::Stop);
        assert_eq!(keys(&message), ["type"]);
        assert_eq!(field(&message, "type"), "stop");
    }
}
