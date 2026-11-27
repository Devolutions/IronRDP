//! An asynchronous H.264 decoder, which returns pictures after `submit` returns, drives the
//! graphics pipeline client through `poll_decoded`.

use core::time::Duration;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ironrdp_core::{Decode as _, MonotonicInstant, ReadCursor, encode_vec};
use ironrdp_dvc::{DvcMessage, DvcProcessor as _};
use ironrdp_egfx::client::{BitmapUpdate, DecodeTimeouts, GraphicsPipelineClient, GraphicsPipelineHandler};
use ironrdp_egfx::decode::{
    DecodeStatus, DecodedFrame, DecodedPicture, DecoderError, DecoderResult, H264Decoder, Yuv420Frame,
};
use ironrdp_egfx::pdu::{
    Avc420Region, CacheToSurfacePdu, CapabilitiesConfirmPdu, CapabilitiesV107Flags, CapabilitySet, Codec1Type, Color,
    CreateSurfacePdu, DeleteSurfacePdu, EndFramePdu, GfxPdu, MapSurfaceToOutputPdu, PixelFormat, Point,
    ResetGraphicsPdu, SolidFillPdu, StartFramePdu, SurfaceToCachePdu, Timestamp, WireToSurface1Pdu,
    encode_avc420_bitmap_stream,
};
use ironrdp_graphics::zgfx::wrap_uncompressed;
use ironrdp_pdu::geometry::ExclusiveRectangle;

const FLUSH_AFTER: Duration = Duration::from_millis(10);
const GIVE_UP_AFTER: Duration = Duration::from_millis(100);

/// What the pipeline asked of the decoders and showed to the handler, in order.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn push(&self, event: String) {
        self.0.lock().unwrap().push(event);
    }

    fn take(&self) -> Vec<String> {
        core::mem::take(&mut *self.0.lock().unwrap())
    }
}

struct LoggingHandler(Log);

impl GraphicsPipelineHandler for LoggingHandler {
    fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
        self.0.push(format!("paint {}", update.data[0]));
    }

    fn on_solid_fill(&mut self, pdu: &SolidFillPdu) {
        self.0.push(format!("fill {}", pdu.fill_pixel.r));
    }

    fn on_surface_deleted(&mut self, surface_id: u16) {
        self.0.push(format!("delete {surface_id}"));
    }

    fn on_frame_complete(&mut self, frame_id: u32) {
        self.0.push(format!("end {frame_id}"));
    }
}

#[derive(Default)]
struct DecoderState {
    /// Tags of the pictures submitted and not yet returned, oldest first.
    decoding: VecDeque<u8>,
    returned: VecDeque<DecoderResult<DecodedPicture>>,
    /// Return the next submission at once instead of leaving it pending.
    decode_next_at_once: bool,
    /// Return a 4096x2048 picture instead of a 16x16 one.
    large: bool,
}

/// A decoder whose pictures stay pending until the test returns them, or until it is flushed.
/// Each picture is a frame whose red channel is the last byte of the H.264 data, its tag.
#[derive(Clone)]
struct AsyncDecoder {
    name: &'static str,
    log: Log,
    state: Arc<Mutex<DecoderState>>,
}

impl AsyncDecoder {
    fn new(log: &Log) -> Self {
        Self::named("", log)
    }

    fn named(name: &'static str, log: &Log) -> Self {
        Self {
            name,
            log: log.clone(),
            state: Arc::default(),
        }
    }

    /// Returns the oldest pending picture.
    fn finish_next(&self) {
        let mut state = self.state.lock().unwrap();
        let tag = state.decoding.pop_front().expect("a pending picture");
        let picture = picture(tag, state.large);
        state.returned.push_back(Ok(picture));
    }

    /// Fails the oldest pending picture.
    fn fail_next(&self) {
        let mut state = self.state.lock().unwrap();
        state.decoding.pop_front().expect("a pending picture");
        state.returned.push_back(Err(DecoderError::msg("corrupt picture")));
    }

    fn decode_next_at_once(&self) {
        self.state.lock().unwrap().decode_next_at_once = true;
    }

    fn return_large_pictures(&self) {
        self.state.lock().unwrap().large = true;
    }
}

impl H264Decoder for AsyncDecoder {
    fn decode(&mut self, _data: &[u8]) -> DecoderResult<DecodedFrame> {
        Err(DecoderError::msg("this decoder only decodes asynchronously"))
    }

    fn submit(&mut self, data: &[u8]) -> DecoderResult<DecodeStatus> {
        let tag = *data.last().expect("H.264 data");
        self.log.push(format!("submit{} {tag}", self.name));
        let mut state = self.state.lock().unwrap();
        if core::mem::take(&mut state.decode_next_at_once) {
            return Ok(DecodeStatus::Ready(picture(tag, state.large)));
        }
        state.decoding.push_back(tag);
        Ok(DecodeStatus::Pending)
    }

    fn poll_decoded(&mut self) -> Option<DecoderResult<DecodedPicture>> {
        self.state.lock().unwrap().returned.pop_front()
    }

    fn flush(&mut self) {
        self.log.push(format!("flush{}", self.name));
        let mut state = self.state.lock().unwrap();
        while let Some(tag) = state.decoding.pop_front() {
            let picture = picture(tag, state.large);
            state.returned.push_back(Ok(picture));
        }
    }

    fn reset(&mut self) {
        self.log.push(format!("reset{}", self.name));
        let mut state = self.state.lock().unwrap();
        state.decoding.clear();
        state.returned.clear();
    }
}

/// An asynchronous decoder that never returns a picture, even when flushed.
struct SilentDecoder(Log);

impl H264Decoder for SilentDecoder {
    fn decode(&mut self, _data: &[u8]) -> DecoderResult<DecodedFrame> {
        Err(DecoderError::msg("this decoder only decodes asynchronously"))
    }

    fn submit(&mut self, data: &[u8]) -> DecoderResult<DecodeStatus> {
        self.0.push(format!("submit {}", data.last().expect("H.264 data")));
        Ok(DecodeStatus::Pending)
    }

    fn flush(&mut self) {
        self.0.push("flush".to_owned());
    }

    fn reset(&mut self) {
        self.0.push("reset".to_owned());
    }
}

fn picture(tag: u8, large: bool) -> DecodedPicture {
    let (width, height) = if large { (4096, 2048) } else { (16, 16) };
    let mut data = vec![0xFF; width * height * 4];
    for pixel in data.chunks_exact_mut(4) {
        pixel[0] = tag;
    }
    DecodedFrame::new(data, u32::try_from(width).unwrap(), u32::try_from(height).unwrap()).into()
}

fn at(milliseconds: u64) -> MonotonicInstant {
    MonotonicInstant::from_millis(milliseconds)
}

fn confirm() -> GfxPdu {
    GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::from_typed(&CapabilitySet::V10_7 {
        flags: CapabilitiesV107Flags::empty(),
    }))
}

/// An active client with a 16x16 surface 1, its log emptied.
fn active_client(log: &Log, decoder: Box<dyn H264Decoder>) -> GraphicsPipelineClient {
    let mut client = GraphicsPipelineClient::new(Box::new(LoggingHandler(log.clone())), Some(decoder))
        .with_decode_timeouts(DecodeTimeouts::new(FLUSH_AFTER, GIVE_UP_AFTER));
    client.start(0).expect("capabilities advertisement");
    send(&mut client, &[confirm(), create_surface(1)]);
    log.take();
    client
}

/// Processes `pdus`, one channel message each, and returns the frame acknowledgements sent.
fn send(client: &mut GraphicsPipelineClient, pdus: &[GfxPdu]) -> Vec<u32> {
    let mut acks = Vec::new();
    for pdu in pdus {
        let messages = client
            .process(0, &wrap_uncompressed(&encode_vec(pdu).expect("encodable PDU")))
            .expect("the client should accept the PDU");
        acks.extend(frame_acks(&messages));
    }
    acks
}

fn poll(client: &mut GraphicsPipelineClient, now: MonotonicInstant) -> Vec<u32> {
    frame_acks(&client.poll_decoded(now).expect("the held commands apply"))
}

fn frame_acks(messages: &[DvcMessage]) -> Vec<u32> {
    messages
        .iter()
        .filter_map(|message| {
            let buf = encode_vec(message.as_ref()).expect("encodable message");
            match GfxPdu::decode(&mut ReadCursor::new(&buf)).expect("a graphics PDU") {
                GfxPdu::FrameAcknowledge(ack) => Some(ack.frame_id),
                _ => None,
            }
        })
        .collect()
}

fn create_surface(surface_id: u16) -> GfxPdu {
    GfxPdu::CreateSurface(CreateSurfacePdu {
        surface_id,
        width: 16,
        height: 16,
        pixel_format: PixelFormat::XRgb,
    })
}

fn start_frame(frame_id: u32) -> GfxPdu {
    GfxPdu::StartFrame(StartFramePdu {
        timestamp: Timestamp {
            milliseconds: 0,
            seconds: 0,
            minutes: 0,
            hours: 0,
        },
        frame_id,
    })
}

fn end_frame(frame_id: u32) -> GfxPdu {
    GfxPdu::EndFrame(EndFramePdu { frame_id })
}

fn full_surface() -> ExclusiveRectangle {
    ExclusiveRectangle {
        left: 0,
        top: 0,
        right: 16,
        bottom: 16,
    }
}

/// An AVC420 update of all of `surface_id` whose picture is tagged `tag`.
fn avc420_on(surface_id: u16, tag: u8) -> GfxPdu {
    GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id,
        codec_id: Codec1Type::Avc420,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: full_surface(),
        bitmap_data: encode_avc420_bitmap_stream(&[Avc420Region::full_frame(16, 16, 22)], &[0, 0, 0, 1, 0x65, tag]),
    })
}

fn avc420(tag: u8) -> GfxPdu {
    avc420_on(1, tag)
}

fn solid_fill(red: u8) -> GfxPdu {
    GfxPdu::SolidFill(SolidFillPdu {
        surface_id: 1,
        fill_pixel: Color {
            b: 0,
            g: 0,
            r: red,
            xa: 0xFF,
        },
        rectangles: vec![full_surface()],
    })
}

/// A frame is acknowledged once its commands are decoded, so not before its picture is out.
// oracle: spec MS-RDPEGFX §2.2.2.13 (the client acknowledges that all the commands of the frame were decoded)
#[test]
fn a_frame_is_acknowledged_once_its_picture_is_out() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder.clone()));

    let acks_while_decoding = send(&mut client, &[start_frame(7), avc420(1), end_frame(7)]);
    decoder.finish_next();
    let acks_once_decoded = poll(&mut client, at(0));

    assert_eq!(acks_while_decoding, Vec::<u32>::new());
    assert_eq!(acks_once_decoded, vec![7]);
    assert_eq!(client.total_frames_decoded(), 1);
}

/// A command after a picture still decoding paints after it, so the picture never covers what
/// the server drew over it.
// oracle: spec MS-RDPEGFX §3.3.5 (the client processes the graphics commands in the order received)
#[test]
fn commands_behind_a_decoding_picture_apply_after_it() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder.clone()));

    send(&mut client, &[start_frame(1), avc420(1), solid_fill(9), end_frame(1)]);
    let before = log.take();
    decoder.finish_next();
    poll(&mut client, at(0));

    assert_eq!(before, ["submit 1"]);
    assert_eq!(log.take(), ["paint 1", "fill 9", "end 1"]);
}

/// A picture the decoder returns at once still waits for an earlier one still decoding.
// oracle: spec MS-RDPEGFX §3.3.5 (the client processes the graphics commands in the order received)
#[test]
fn a_picture_decoded_at_once_paints_after_an_earlier_one() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder.clone()));

    send(&mut client, &[start_frame(1), avc420(1), end_frame(1)]);
    decoder.decode_next_at_once();
    send(&mut client, &[start_frame(2), avc420(2), end_frame(2)]);
    log.take();
    decoder.finish_next();
    let acks = poll(&mut client, at(0));

    assert_eq!(log.take(), ["paint 1", "end 1", "paint 2", "end 2"]);
    assert_eq!(acks, vec![1, 2]);
}

/// Surfaces with decoders of their own paint in the order their updates arrived, whichever
/// decoder returns first.
// oracle: spec MS-RDPEGFX §3.3.5 (the client processes the graphics commands in the order received)
#[test]
fn surfaces_with_their_own_decoders_paint_in_arrival_order() {
    let log = Log::default();
    let first = AsyncDecoder::named("@a", &log);
    let second = AsyncDecoder::named("@b", &log);
    let factory_decoder = second.clone();
    let mut client = GraphicsPipelineClient::new(Box::new(LoggingHandler(log.clone())), Some(Box::new(first.clone())))
        .with_h264_decoder_factory(Box::new(move || Some(Box::new(factory_decoder.clone()))));
    client.start(0).expect("capabilities advertisement");
    send(&mut client, &[confirm(), create_surface(1), create_surface(2)]);
    send(
        &mut client,
        &[start_frame(1), avc420_on(1, 1), avc420_on(2, 2), end_frame(1)],
    );
    log.take();

    second.finish_next();
    let acks_after_second = poll(&mut client, at(0));
    first.finish_next();
    let acks_after_first = poll(&mut client, at(0));

    assert_eq!(acks_after_second, Vec::<u32>::new());
    assert_eq!(acks_after_first, vec![1]);
    assert_eq!(log.take(), ["paint 1", "paint 2", "end 1"]);
}

/// A cache entry taken behind a pending picture holds that picture's pixels, not those it replaced.
// oracle: spec MS-RDPEGFX §3.3.5 (the client processes the graphics commands in the order received)
#[test]
fn a_cache_entry_taken_behind_a_pending_picture_holds_its_pixels() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder.clone()));
    send(
        &mut client,
        &[
            GfxPdu::ResetGraphics(ResetGraphicsPdu {
                width: 16,
                height: 16,
                monitors: vec![],
            }),
            create_surface(1),
            GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
                surface_id: 1,
                output_origin_x: 0,
                output_origin_y: 0,
            }),
        ],
    );
    let _ = client.drain_output();

    send(
        &mut client,
        &[
            start_frame(1),
            avc420(7),
            GfxPdu::SurfaceToCache(SurfaceToCachePdu {
                surface_id: 1,
                cache_key: 1,
                cache_slot: 1,
                source_rectangle: full_surface(),
            }),
            solid_fill(9),
            GfxPdu::CacheToSurface(CacheToSurfacePdu {
                cache_slot: 1,
                surface_id: 1,
                destination_points: vec![Point { x: 0, y: 0 }],
            }),
            end_frame(1),
        ],
    );
    decoder.finish_next();
    poll(&mut client, at(0));
    let output = client.drain_output();

    let last = output.last().expect("the frame changed the output");
    assert_eq!(last.data[0], 7, "the cached picture is pasted over the fill");
}

/// A picture of a later frame reaches the decoder while the earlier one is still decoding, so the
/// decoder works ahead instead of one picture at a time.
// oracle: mutant AVC420 updates behind a pending picture held undecoded until it is out
#[test]
fn pictures_of_later_frames_are_submitted_while_earlier_ones_decode() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder));

    send(
        &mut client,
        &[
            start_frame(1),
            avc420(1),
            end_frame(1),
            start_frame(2),
            avc420(2),
            end_frame(2),
        ],
    );

    assert_eq!(log.take(), ["submit 1", "submit 2"]);
}

/// A reset arriving while a picture decodes waits for it, so the decoder is not reset while the
/// pipeline waits for one of its pictures, and the next stream's pictures go to the reset decoder.
// oracle: mutant the decoder reset, or the next stream submitted, before the pending picture is out
#[test]
fn reset_graphics_waits_for_the_pictures_before_it() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder.clone()));

    send(
        &mut client,
        &[
            start_frame(1),
            avc420(1),
            end_frame(1),
            GfxPdu::ResetGraphics(ResetGraphicsPdu {
                width: 16,
                height: 16,
                monitors: vec![],
            }),
            create_surface(1),
            start_frame(2),
            avc420(2),
            end_frame(2),
        ],
    );
    let before = log.take();
    decoder.finish_next();
    poll(&mut client, at(0));
    let during = log.take();

    assert_eq!(before, ["submit 1"]);
    assert_eq!(during, ["paint 1", "end 1", "reset", "submit 2"]);
}

/// A surface deleted while one of its pictures decodes is deleted after the picture paints, so
/// commands between them still read the picture.
// oracle: spec MS-RDPEGFX §3.3.5 (the client processes the graphics commands in the order received)
#[test]
fn a_surface_is_deleted_after_its_pending_picture() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder.clone()));

    send(
        &mut client,
        &[
            start_frame(1),
            avc420(1),
            GfxPdu::DeleteSurface(DeleteSurfacePdu { surface_id: 1 }),
            end_frame(1),
        ],
    );
    decoder.finish_next();
    poll(&mut client, at(0));

    assert_eq!(log.take(), ["submit 1", "paint 1", "delete 1", "end 1"]);
}

/// A picture the decoder fails is dropped like a synchronous decode failure: its frame is still
/// acknowledged, so the server is not left waiting.
// oracle: mutant a failed picture blocking the queue or leaving its frame unacknowledged
#[test]
fn a_failed_picture_is_dropped_and_its_frame_acknowledged() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder.clone()));

    send(
        &mut client,
        &[
            start_frame(1),
            avc420(1),
            end_frame(1),
            start_frame(2),
            avc420(2),
            end_frame(2),
        ],
    );
    decoder.fail_next();
    decoder.finish_next();
    let acks = poll(&mut client, at(0));

    assert_eq!(acks, vec![1, 2]);
    assert_eq!(client.dropped_updates(), 1);
}

/// A protocol reset discards the pictures still decoding with everything else, so the pipeline
/// does not wait for pictures its reset decoder will never return.
// oracle: mutant pending pictures kept across a protocol reset
#[test]
fn a_protocol_reset_discards_pending_pictures() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder));

    send(&mut client, &[start_frame(1), avc420(1), end_frame(1)]);
    let reset = client.request_protocol_reset();

    assert!(reset.is_some());
    assert!(!client.has_pending_decodes());
}

/// Channel data arriving after a picture was returned paints it, without a separate `poll_decoded`.
// oracle: mutant returned pictures painted only by `poll_decoded`
#[test]
fn processing_channel_data_paints_returned_pictures() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder.clone()));

    send(&mut client, &[start_frame(1), avc420(1)]);
    decoder.finish_next();
    let acks = send(&mut client, &[end_frame(1)]);

    assert_eq!(acks, vec![1]);
}

/// When the server sends one update and then nothing, a decoder holding the picture back for more
/// input is flushed, so the update still paints and its frame is acknowledged.
// oracle: mutant no flush of a decoder holding back the last picture of an idle stream
#[test]
fn an_idle_stream_flushes_the_decoder_holding_its_last_picture() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder));

    send(&mut client, &[start_frame(1), avc420(1), end_frame(1)]);
    let acks_at_first_poll = poll(&mut client, at(1000));
    let acks_once_late = poll(&mut client, at(1000) + FLUSH_AFTER);

    assert_eq!(acks_at_first_poll, Vec::<u32>::new());
    assert_eq!(acks_once_late, vec![1]);
    assert_eq!(log.take(), ["submit 1", "flush", "paint 1", "end 1"]);
}

/// When the server's acknowledgement window is full, it waits for acknowledgements before sending
/// more, so the pictures the decoder holds back are flushed instead of waiting for input.
// oracle: mutant no flush while every frame of a full acknowledgement window is held
#[test]
fn a_full_acknowledgement_window_flushes_the_decoder() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    let mut client = active_client(&log, Box::new(decoder));

    send(
        &mut client,
        &[
            start_frame(1),
            avc420(1),
            end_frame(1),
            start_frame(2),
            avc420(2),
            end_frame(2),
        ],
    );
    poll(&mut client, at(0));
    let acks = poll(&mut client, at(0) + FLUSH_AFTER);

    assert_eq!(acks, vec![1, 2]);
}

/// A decoder that never returns a picture, even when flushed, is given up at the deadline: the
/// update is dropped, its frame acknowledged, and the decoder reset, so the session goes on.
// oracle: mutant a pending picture waited for without a deadline
#[test]
fn a_decoder_that_never_answers_is_given_up_at_the_deadline() {
    let log = Log::default();
    let mut client = active_client(&log, Box::new(SilentDecoder(log.clone())));

    send(&mut client, &[start_frame(1), avc420(1), end_frame(1), solid_fill(9)]);
    poll(&mut client, at(0));
    let acks_before_deadline = poll(&mut client, at(0) + (GIVE_UP_AFTER - Duration::from_millis(1)));
    let acks_at_deadline = poll(&mut client, at(0) + GIVE_UP_AFTER);

    assert_eq!(acks_before_deadline, Vec::<u32>::new());
    assert_eq!(acks_at_deadline, vec![1]);
    assert_eq!(client.dropped_updates(), 1);
    assert!(!client.has_pending_decodes());
    assert_eq!(log.take(), ["submit 1", "flush", "reset", "end 1", "fill 9"]);
}

/// More pictures pending than the budget allows are given up at once rather than queued.
// oracle: mutant pending pictures unbounded
#[test]
fn pending_pictures_past_the_budget_are_given_up() {
    let log = Log::default();
    let mut client = active_client(&log, Box::new(SilentDecoder(log.clone())));

    let mut acks = Vec::new();
    for frame_id in 0..17 {
        acks.extend(send(
            &mut client,
            &[
                start_frame(frame_id),
                avc420(u8::try_from(frame_id).unwrap()),
                end_frame(frame_id),
            ],
        ));
    }

    assert_eq!(acks, (0..17).collect::<Vec<_>>());
    assert!(!client.has_pending_decodes());
    assert_eq!(client.dropped_updates(), 17);
}

/// Held commands larger than their budget give up the pending picture, so the queue drains instead
/// of retaining their payloads.
// oracle: mutant held command payloads unbounded
#[test]
fn held_command_bytes_past_the_budget_give_up_the_pending_picture() {
    let log = Log::default();
    let mut client = active_client(&log, Box::new(SilentDecoder(log.clone())));
    send(&mut client, &[start_frame(1), avc420(1)]);

    let big = GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id: 1,
        codec_id: Codec1Type::ClearCodec,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: full_surface(),
        bitmap_data: vec![0xEE; 4 << 20],
    });
    for _ in 0..9 {
        send(&mut client, core::slice::from_ref(&big));
    }

    assert!(!client.has_pending_decodes());
    assert!(log.take().contains(&"reset".to_owned()));
}

/// Returned pictures waiting behind a pending one, larger than their budget, give the pending
/// picture up, so the queue drains instead of retaining them.
// oracle: mutant returned pictures held behind a pending one unbounded
#[test]
fn returned_picture_bytes_past_the_budget_give_up_the_pending_picture() {
    let log = Log::default();
    let decoder = AsyncDecoder::new(&log);
    decoder.return_large_pictures();
    let mut client = active_client(&log, Box::new(decoder.clone()));
    send(&mut client, &[start_frame(0), avc420(0), end_frame(0)]);

    let mut acks = Vec::new();
    for frame_id in 1..6 {
        decoder.decode_next_at_once();
        acks.extend(send(
            &mut client,
            &[
                start_frame(frame_id),
                avc420(u8::try_from(frame_id).unwrap()),
                end_frame(frame_id),
            ],
        ));
    }

    assert_eq!(acks, (0..6).collect::<Vec<_>>());
    assert!(log.take().contains(&"reset".to_owned()));
}

/// A YUV 4:2:0 picture is converted with the full-range BT.709 transform of MS-RDPEGFX, whatever
/// the decoder would convert with.
// oracle: spec MS-RDPEGFX §3.3.8.3.1 (Y=128, U=128, V=228: R=128+1.5748*100 clamps to 255, G=128-0.4681*100=81.19, B=128)
#[test]
fn a_yuv_picture_is_converted_with_the_rdp_transform() {
    struct YuvDecoder;
    impl H264Decoder for YuvDecoder {
        fn decode(&mut self, _data: &[u8]) -> DecoderResult<DecodedFrame> {
            Err(DecoderError::msg("planes only"))
        }

        fn submit(&mut self, _data: &[u8]) -> DecoderResult<DecodeStatus> {
            let picture = Yuv420Frame::new(16, 16, vec![128; 256], 16, vec![128; 64], vec![228; 64], 8)?;
            Ok(DecodeStatus::Ready(picture.into()))
        }
    }
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl GraphicsPipelineHandler for Capture {
        fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
            self.0.lock().unwrap().extend_from_slice(&update.data[..4]);
        }
    }
    let captured = Arc::new(Mutex::new(Vec::new()));
    let mut client = GraphicsPipelineClient::new(Box::new(Capture(Arc::clone(&captured))), Some(Box::new(YuvDecoder)));
    client.start(0).expect("capabilities advertisement");
    send(&mut client, &[confirm(), create_surface(1), avc420(1)]);

    assert_eq!(*captured.lock().unwrap(), [255, 81, 128, 255]);
}

/// Large pictures other surfaces' decoders returned while the oldest surface's picture is pending
/// count against the budget, so they give the pending picture up instead of piling up unpolled.
// oracle: mutant pictures returned behind a pending one of another decoder left uncounted
#[test]
fn large_pictures_returned_on_other_surfaces_give_up_the_pending_one() {
    let log = Log::default();
    let first = AsyncDecoder::named("@1", &log);
    let others = Arc::new(Mutex::new(Vec::new()));
    let (factory_log, factory_others) = (log.clone(), Arc::clone(&others));
    let mut client = GraphicsPipelineClient::new(Box::new(LoggingHandler(log.clone())), Some(Box::new(first)))
        .with_h264_decoder_factory(Box::new(move || {
            let decoder = AsyncDecoder::named("@other", &factory_log);
            decoder.return_large_pictures();
            factory_others.lock().unwrap().push(decoder.clone());
            Some(Box::new(decoder))
        }));
    client.start(0).expect("capabilities advertisement");
    send(&mut client, &[confirm()]);
    for surface_id in 1..=6 {
        send(&mut client, &[create_surface(surface_id)]);
    }
    let mut frame = vec![start_frame(1)];
    frame.extend((1..=6).map(|surface_id| avc420_on(surface_id, u8::try_from(surface_id).unwrap())));
    frame.push(end_frame(1));
    send(&mut client, &frame);
    log.take();

    for decoder in others.lock().unwrap().iter() {
        decoder.finish_next();
    }
    let acks = poll(&mut client, at(0));

    assert_eq!(acks, vec![1]);
    assert!(!client.has_pending_decodes());
    assert!(log.take().contains(&"reset@1".to_owned()));
    assert_eq!(client.dropped_updates(), 1);
}
