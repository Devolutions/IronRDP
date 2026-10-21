use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_egfx::decode::{DecodedFrame, DecoderError, DecoderResult, H264Decoder};
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_rdpegt::{GeometryRect, MappedGeometry, RegionData};
use ironrdp_rdpevor::pdu::{
    ClientNotification, MF_VIDEO_FORMAT_H264, Notification, PresentationCommand, PresentationRequest,
    PresentationResponse, VideoData,
};
use ironrdp_rdpevor::{VideoControlClient, VideoDataClient, VideoPresenter};

const MAPPING_ID: u64 = 0x8000_7ABA_0004_0222;

/// MS-RDPEVOR 4.1, followed by its reserved byte.
const START_FIXTURE: &str = "69000000 01000000 0301011D C0120000 E0010000 F4000000 E0010000 F4000000 \
                             A47A3B82 0F000000 22020400 BA7A0080 48323634 00001000 800000AA 00389B71 \
                             25000000 00000001 6742C015 95A07821 F9E10000 03000100 0003003C 0DA08846 \
                             A0000000 0168CE3C 8000";

/// MS-RDPEVOR 4.4, followed by its reserved byte.
const STOP_FIXTURE: &str = "44000000 01000000 03010200 00000000 00000000 00000000 00000000 00000000 \
                            00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 \
                            00000000 00";

/// Header of MS-RDPEVOR 4.3.
const VIDEO_DATA_HEADER: &str = "33030000 04000000 03010300 C7C60600 00000000 00000000 00000000 01000100 \
                                 01000000 0B030000";

/// MS-RDPEVOR 4.3: one complete 480x244 H.264 key frame.
const VIDEO_DATA_EXAMPLE: &[u8] = include_bytes!("../../test_data/rdpevor/video_data_example.bin");

fn bytes(fixture: &str) -> Vec<u8> {
    hex::decode(fixture.replace(' ', "")).unwrap()
}

fn sequence_header() -> Vec<u8> {
    bytes("00000001 6742C015 95A07821 F9E10000 03000100 0003003C 0DA08846 A0000000 0168CE3C 80")
}

fn start_request() -> PresentationRequest {
    PresentationRequest {
        presentation_id: 3,
        version: 1,
        command: PresentationCommand::START,
        frame_rate: 29,
        average_bitrate_kbps: 4800,
        source_width: 480,
        source_height: 244,
        scaled_width: 480,
        scaled_height: 244,
        timestamp_offset: 66_609_445_540,
        geometry_mapping_id: MAPPING_ID,
        video_subtype: MF_VIDEO_FORMAT_H264,
        extra_data: sequence_header(),
    }
}

#[test]
fn messages_match_the_specification_examples() {
    let start = bytes(START_FIXTURE);
    assert_eq!(decode::<PresentationRequest>(&start).unwrap(), start_request());
    // The reserved trailing byte is not counted by cbSize.
    assert_eq!(encode_vec(&start_request()).unwrap(), start[..105]);

    let response = PresentationResponse { presentation_id: 3 };
    assert_eq!(encode_vec(&response).unwrap(), bytes("0C000000 02000000 03000000"));
    assert_eq!(
        decode::<PresentationResponse>(&bytes("0C000000 02000000 03000000")).unwrap(),
        response
    );

    let stop = bytes(STOP_FIXTURE);
    let stop_request = PresentationRequest {
        presentation_id: 3,
        version: 1,
        command: PresentationCommand::STOP,
        frame_rate: 0,
        average_bitrate_kbps: 0,
        source_width: 0,
        source_height: 0,
        scaled_width: 0,
        scaled_height: 0,
        timestamp_offset: 0,
        geometry_mapping_id: 0,
        video_subtype: [0; 16],
        extra_data: Vec::new(),
    };
    assert_eq!(decode::<PresentationRequest>(&stop).unwrap(), stop_request);
    assert_eq!(encode_vec(&stop_request).unwrap(), stop[..68]);

    let video_data = VIDEO_DATA_EXAMPLE;
    assert_eq!(video_data[..40], bytes(VIDEO_DATA_HEADER));
    let sample = &video_data[40..];
    let decoded = decode::<VideoData<'_>>(video_data).unwrap();
    assert_eq!(
        decoded,
        VideoData {
            presentation_id: 3,
            version: 1,
            flags: VideoData::HAS_TIMESTAMPS | VideoData::KEYFRAME,
            timestamp: 444_103,
            duration: 0,
            current_packet_index: 1,
            packets_in_sample: 1,
            sample_number: 1,
            sample,
        }
    );
    assert_eq!(encode_vec(&decoded).unwrap(), video_data);
}

#[cfg(feature = "openh264-bundled")]
#[test]
fn specification_sample_is_decoded_with_openh264_and_placed_at_its_geometry() {
    present_specification_sample(Box::new(|| {
        let decoder: Box<dyn H264Decoder> = Box::new(
            ironrdp_egfx::decode::OpenH264Decoder::new()
                .ok()?
                .with_yuv_matrix(ironrdp_egfx::decode::YuvMatrix::Bt601Limited),
        );
        Some(decoder)
    }));
}

#[cfg(windows)]
#[test]
fn specification_sample_is_decoded_with_media_foundation_and_placed_at_its_geometry() {
    use ironrdp_egfx_native::{MediaFoundationH264Decoder, YuvMatrix};

    // The decoder pads the 480x244 picture to whole macroblocks.
    let mut decoder = MediaFoundationH264Decoder::new(YuvMatrix::Bt601Limited).unwrap();
    let frame = decoder.decode(&VIDEO_DATA_EXAMPLE[40..]).unwrap();
    assert_eq!((frame.width(), frame.height()), (480, 256));
    assert_eq!(frame.data().len(), 480 * 256 * 4);
    // A second key frame decodes on the same stream.
    decoder.decode(&VIDEO_DATA_EXAMPLE[40..]).unwrap();

    present_specification_sample(Box::new(|| {
        let decoder: Box<dyn H264Decoder> = Box::new(MediaFoundationH264Decoder::new(YuvMatrix::Bt601Limited).ok()?);
        Some(decoder)
    }));
}

#[cfg(all(windows, feature = "openh264-bundled"))]
#[test]
fn media_foundation_and_openh264_agree_on_the_specification_sample() {
    use ironrdp_egfx::decode::OpenH264Decoder;
    use ironrdp_egfx_native::{MediaFoundationH264Decoder, YuvMatrix};

    let sample = &VIDEO_DATA_EXAMPLE[40..];
    let row = |frame: &DecodedFrame, y: usize| {
        let stride = usize::try_from(frame.width()).unwrap() * 4;
        frame.data()[y * stride..][..480 * 4].to_vec()
    };
    let mean_difference = |left: &DecodedFrame, right: &DecodedFrame| {
        let mut total = 0u64;
        for y in 0..244 {
            for (left, right) in row(left, y).iter().zip(row(right, y)) {
                total += u64::from(left.abs_diff(right));
            }
        }
        total / (480 * 244 * 4)
    };
    let openh264 = |matrix| {
        OpenH264Decoder::new()
            .unwrap()
            .with_yuv_matrix(matrix)
            .decode(sample)
            .unwrap()
    };
    let media_foundation = |matrix| MediaFoundationH264Decoder::new(matrix).unwrap().decode(sample).unwrap();

    // The two conversions differ enough for a mismatch to show.
    assert!(mean_difference(&openh264(YuvMatrix::Bt601Limited), &openh264(YuvMatrix::Bt709Full)) >= 4);
    for matrix in [YuvMatrix::Bt601Limited, YuvMatrix::Bt709Full] {
        let mean = mean_difference(&openh264(matrix), &media_foundation(matrix));
        assert!(mean <= 2, "{matrix:?}: mean channel difference {mean}");
    }
}

#[cfg(any(windows, feature = "openh264-bundled"))]
fn present_specification_sample(decoder_factory: ironrdp_rdpevor::DecoderFactory) {
    let mut presenter = VideoPresenter::new(decoder_factory, Box::new(|| 1_000_000));
    assert!(presenter.start(&start_request()));
    // MS-RDPEGT 4.1 describes the geometry of this presentation.
    presenter.set_geometry(
        MAPPING_ID,
        Some(MappedGeometry {
            top_level_id: 0x0003_01E2,
            rect: GeometryRect {
                left: 16,
                top: 138,
                right: 496,
                bottom: 382,
            },
            top_level_rect: GeometryRect {
                left: 291,
                top: 114,
                right: 1144,
                bottom: 714,
            },
            region: None,
        }),
    );
    presenter.push_packet(&decode::<VideoData<'_>>(VIDEO_DATA_EXAMPLE).unwrap());
    let updates = presenter.poll(1920, 1080);
    assert_eq!(updates.len(), 1);
    assert_eq!(
        updates[0].region,
        ExclusiveRectangle {
            left: 307,
            top: 252,
            right: 787,
            bottom: 496,
        }
    );
    assert_eq!(updates[0].data.len(), 480 * 244 * 4);
    assert!(presenter.take_notifications().is_empty());
}

#[test]
fn client_notifications_round_trip() {
    let network_error = ClientNotification {
        presentation_id: 3,
        notification: Notification::NetworkError,
    };
    let encoded = encode_vec(&network_error).unwrap();
    assert_eq!(encoded, bytes("10000000 03000000 03010000 00000000"));
    assert_eq!(decode::<ClientNotification>(&encoded).unwrap(), network_error);

    let override_rate = ClientNotification {
        presentation_id: 3,
        notification: Notification::FrameRateOverride {
            flags: Notification::OVERRIDE_FRAME_RATE,
            desired_frame_rate: 24,
        },
    };
    let encoded = encode_vec(&override_rate).unwrap();
    assert_eq!(
        encoded,
        bytes("20000000 03000000 03020000 10000000 02000000 18000000 00000000 00000000")
    );
    assert_eq!(decode::<ClientNotification>(&encoded).unwrap(), override_rate);
}

#[test]
fn malformed_messages_are_rejected() {
    let mut start = bytes(START_FIXTURE);
    // cbSize larger than the received data.
    start[0] = 0xFF;
    assert!(decode::<PresentationRequest>(&start).is_err());
    // cbExtra larger than the message.
    let mut start = bytes(START_FIXTURE);
    start[64] = 0xFF;
    assert!(decode::<PresentationRequest>(&start).is_err());
    // Wrong packet type for the structure.
    assert!(decode::<VideoData<'_>>(&bytes(START_FIXTURE)).is_err());

    // Packet index outside the sample.
    let mut video_data = bytes(VIDEO_DATA_HEADER);
    video_data.resize(video_data.len() + 779, 0);
    video_data[28] = 2;
    assert!(decode::<VideoData<'_>>(&video_data).is_err());
    video_data[28] = 0;
    assert!(decode::<VideoData<'_>>(&video_data).is_err());
}

/// Decoder returning a picture whose pixels encode their coordinates.
struct FakeDecoder {
    width: u32,
    height: u32,
    decoded: Arc<AtomicUsize>,
    inputs: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl H264Decoder for FakeDecoder {
    fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame> {
        self.inputs.lock().unwrap().push(data.to_vec());
        if data.ends_with(b"corrupt") {
            return Err(DecoderError::msg("corrupt picture"));
        }
        self.decoded.fetch_add(1, Ordering::SeqCst);
        let mut pixels = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                pixels.extend_from_slice(&[u8::try_from(x).unwrap(), u8::try_from(y).unwrap(), 0xAA, 0xFF]);
            }
        }
        Ok(DecodedFrame::new(pixels, self.width, self.height))
    }
}

struct Harness {
    presenter: Arc<Mutex<VideoPresenter>>,
    clock: Arc<AtomicU64>,
    decoded: Arc<AtomicUsize>,
    inputs: Arc<Mutex<Vec<Vec<u8>>>>,
    created: Arc<AtomicUsize>,
}

impl Harness {
    /// Presenter whose decoder pads a 4x2 picture to 16x16 macroblocks.
    fn new() -> Self {
        let clock = Arc::new(AtomicU64::new(1_000_000_000));
        let decoded = Arc::new(AtomicUsize::new(0));
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let created = Arc::new(AtomicUsize::new(0));
        let (factory_decoded, factory_inputs) = (Arc::clone(&decoded), Arc::clone(&inputs));
        let factory_created = Arc::clone(&created);
        let now = Arc::clone(&clock);
        let presenter = VideoPresenter::new(
            Box::new(move || {
                factory_created.fetch_add(1, Ordering::SeqCst);
                let fake: Box<dyn H264Decoder> = Box::new(FakeDecoder {
                    width: 16,
                    height: 16,
                    decoded: Arc::clone(&factory_decoded),
                    inputs: Arc::clone(&factory_inputs),
                });
                Some(fake)
            }),
            Box::new(move || now.load(Ordering::SeqCst)),
        );
        Self {
            presenter: Arc::new(Mutex::new(presenter)),
            clock,
            decoded,
            inputs,
            created,
        }
    }

    fn advance(&self, hns: u64) {
        self.clock.fetch_add(hns, Ordering::SeqCst);
    }

    fn presenter(&self) -> std::sync::MutexGuard<'_, VideoPresenter> {
        self.presenter.lock().unwrap()
    }
}

fn small_request() -> PresentationRequest {
    PresentationRequest {
        source_width: 8,
        source_height: 4,
        scaled_width: 4,
        scaled_height: 2,
        ..start_request()
    }
}

/// Tracked rectangle at (10, 20), 8x4, with two visible parts.
fn small_geometry() -> MappedGeometry {
    MappedGeometry {
        top_level_id: 0,
        rect: GeometryRect {
            left: 2,
            top: 4,
            right: 10,
            bottom: 8,
        },
        top_level_rect: GeometryRect {
            left: 8,
            top: 16,
            right: 100,
            bottom: 100,
        },
        region: Some(RegionData {
            bound: GeometryRect::default(),
            rects: vec![
                GeometryRect {
                    left: 0,
                    top: 0,
                    right: 8,
                    bottom: 2,
                },
                GeometryRect {
                    left: 6,
                    top: 2,
                    right: 8,
                    bottom: 4,
                },
            ],
        }),
    }
}

fn packet(sample_number: u32, index: u16, packets: u16, flags: u8, timestamp: u64, sample: &[u8]) -> Vec<u8> {
    encode_vec(&VideoData {
        presentation_id: 3,
        version: 1,
        flags,
        timestamp,
        duration: 333_333,
        current_packet_index: index,
        packets_in_sample: packets,
        sample_number,
        sample,
    })
    .unwrap()
}

const KEY: u8 = VideoData::HAS_TIMESTAMPS | VideoData::KEYFRAME;
const DELTA: u8 = VideoData::HAS_TIMESTAMPS;

#[test]
fn control_channel_answers_start_only_when_the_presentation_can_be_decoded() {
    let harness = Harness::new();
    let mut control = VideoControlClient::new(Arc::clone(&harness.presenter));
    control.start(1).unwrap();

    let other_subtype = PresentationRequest {
        video_subtype: [0x11; 16],
        ..small_request()
    };
    assert!(
        control
            .process(1, &encode_vec(&other_subtype).unwrap())
            .unwrap()
            .is_empty()
    );
    assert_eq!(harness.presenter().presentation_id(), None);

    let response = control.process(1, &encode_vec(&small_request()).unwrap()).unwrap();
    assert_eq!(response.len(), 1);
    assert_eq!(
        decode::<PresentationResponse>(&encode_vec(&*response[0]).unwrap()).unwrap(),
        PresentationResponse { presentation_id: 3 }
    );
    assert_eq!(harness.presenter().presentation_id(), Some(3));
    // A repeated start for the active presentation is ignored.
    assert!(
        control
            .process(1, &encode_vec(&small_request()).unwrap())
            .unwrap()
            .is_empty()
    );
    // So is a start for another presentation while this one is active.
    let other_presentation = PresentationRequest {
        presentation_id: 4,
        ..small_request()
    };
    assert!(
        control
            .process(1, &encode_vec(&other_presentation).unwrap())
            .unwrap()
            .is_empty()
    );
    assert_eq!(harness.presenter().presentation_id(), Some(3));

    let stop = PresentationRequest {
        command: PresentationCommand::STOP,
        ..small_request()
    };
    assert!(control.process(1, &encode_vec(&stop).unwrap()).unwrap().is_empty());
    assert_eq!(harness.presenter().presentation_id(), None);

    // Without a decoder, the start is not answered.
    let mut without_decoder = VideoPresenter::new(Box::new(|| None), Box::new(|| 0));
    assert!(!without_decoder.start(&small_request()));
}

#[test]
fn samples_are_reassembled_decoded_scaled_and_clipped_to_the_visible_region() {
    let harness = Harness::new();
    let mut data = VideoDataClient::new(Arc::clone(&harness.presenter));
    assert!(harness.presenter().start(&small_request()));
    harness.presenter().set_geometry(MAPPING_ID, Some(small_geometry()));

    // A key frame without parameter sets, in two packets.
    data.process(2, &packet(1, 1, 2, KEY, 0, &[0, 0, 0, 1, 0x65])).unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 0);
    data.process(2, &packet(1, 2, 2, KEY, 0, &[0x88])).unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 1);
    // The sequence header from the start request precedes the first key frame.
    let mut expected_input = sequence_header();
    expected_input.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88]);
    assert_eq!(harness.inputs.lock().unwrap()[0], expected_input);

    let updates = harness.presenter().poll(1024, 768);
    // The 4x2 picture is scaled 2x to the 8x4 tracked rectangle at (10, 20).
    assert_eq!(
        updates.iter().map(|update| update.region.clone()).collect::<Vec<_>>(),
        [
            ExclusiveRectangle {
                left: 10,
                top: 20,
                right: 18,
                bottom: 22,
            },
            ExclusiveRectangle {
                left: 16,
                top: 22,
                right: 18,
                bottom: 24,
            },
        ]
    );
    let pixel = |data: &[u8], index: usize| (data[index * 4], data[index * 4 + 1]);
    // First row of the first part: source columns 0, 0, 1, 1, 2, 2, 3, 3.
    let first = &updates[0].data;
    assert_eq!(first.len(), 8 * 2 * 4);
    assert_eq!(
        (0..8).map(|index| pixel(first, index)).collect::<Vec<_>>(),
        [(0, 0), (0, 0), (1, 0), (1, 0), (2, 0), (2, 0), (3, 0), (3, 0)]
    );
    // The second part covers source column 3 and row 1.
    assert_eq!(pixel(&updates[1].data, 0), (3, 1));
    assert_eq!(harness.presenter().statistics(), (1, 0));

    // Nothing more is due.
    assert!(harness.presenter().poll(1024, 768).is_empty());
}

#[test]
fn pictures_are_shown_at_their_presentation_time() {
    let harness = Harness::new();
    let mut data = VideoDataClient::new(Arc::clone(&harness.presenter));
    assert!(harness.presenter().start(&small_request()));
    harness.presenter().set_geometry(MAPPING_ID, Some(small_geometry()));

    data.process(2, &packet(1, 1, 1, KEY, 1_000_000, &[0, 0, 1, 0x67, 0x65]))
        .unwrap();
    data.process(2, &packet(2, 1, 1, DELTA, 1_333_333, &[0, 0, 1, 0x41]))
        .unwrap();
    data.process(2, &packet(3, 1, 1, DELTA, 1_666_666, &[0, 0, 1, 0x41]))
        .unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 3);
    // The first picture is due when it arrives.
    assert!(!harness.presenter().poll(1024, 768).is_empty());
    let next = harness.presenter().next_deadline().unwrap();
    assert_eq!(next, harness.clock.load(Ordering::SeqCst) + 333_333);
    assert!(harness.presenter().poll(1024, 768).is_empty());

    // Once both remaining pictures are due, only the latest is shown.
    harness.advance(700_000);
    assert!(!harness.presenter().poll(1024, 768).is_empty());
    assert_eq!(harness.presenter().statistics(), (2, 1));
    assert_eq!(harness.presenter().next_deadline(), None);

    // Without geometry, pictures are not shown.
    harness.presenter().set_geometry(MAPPING_ID, None);
    data.process(2, &packet(4, 1, 1, DELTA, 2_000_000, &[0, 0, 1, 0x41]))
        .unwrap();
    harness.advance(400_000);
    assert!(harness.presenter().poll(1024, 768).is_empty());
}

#[test]
fn lost_packets_request_a_key_frame_and_skip_until_it_arrives() {
    let harness = Harness::new();
    let mut data = VideoDataClient::new(Arc::clone(&harness.presenter));
    let mut control = VideoControlClient::new(Arc::clone(&harness.presenter));
    control.start(1).unwrap();
    control.process(1, &encode_vec(&small_request()).unwrap()).unwrap();

    data.process(2, &packet(1, 1, 1, KEY, 0, &[0, 0, 1, 0x67])).unwrap();
    // The second packet of sample 2 arrives without the first.
    data.process(2, &packet(2, 2, 2, DELTA, 333_333, &[0x41])).unwrap();
    data.process(2, &packet(3, 1, 1, DELTA, 666_666, &[0, 0, 1, 0x41]))
        .unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 1);

    let messages = control.pending_messages().unwrap();
    assert_eq!(messages.len(), 1);
    assert!(control.pending_messages().unwrap().is_empty());

    data.process(2, &packet(4, 1, 1, KEY, 999_999, &[0, 0, 1, 0x67]))
        .unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 2);

    // Decoding failures also ask for a key frame, at most once per second.
    data.process(2, &packet(5, 1, 1, DELTA, 1_333_332, b"\0\0\x01corrupt"))
        .unwrap();
    assert!(control.pending_messages().unwrap().is_empty());
    harness.advance(10_000_000);
    data.process(2, &packet(6, 1, 1, DELTA, 1_666_665, b"\0\0\x01corrupt"))
        .unwrap();
    assert_eq!(control.pending_messages().unwrap().len(), 1);

    // Data for another presentation is ignored.
    let other = encode_vec(&VideoData {
        presentation_id: 9,
        ..decode::<VideoData<'_>>(&packet(7, 1, 1, KEY, 0, &[0, 0, 1, 0x67])).unwrap()
    })
    .unwrap();
    data.process(2, &other).unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 2);

    // Closing the control channel ends the presentation.
    control.close(1);
    assert_eq!(harness.presenter().presentation_id(), None);
}

#[test]
fn a_decoder_that_keeps_failing_is_recreated() {
    let harness = Harness::new();
    let mut data = VideoDataClient::new(Arc::clone(&harness.presenter));
    assert!(harness.presenter().start(&small_request()));
    assert_eq!(harness.created.load(Ordering::SeqCst), 1);
    data.process(2, &packet(1, 1, 1, KEY, 0, &[0, 0, 1, 0x67])).unwrap();

    for sample in 2..5 {
        data.process(2, &packet(sample, 1, 1, DELTA, 0, b"\0\0\x01corrupt"))
            .unwrap();
    }
    assert_eq!(harness.created.load(Ordering::SeqCst), 2);

    // The new decoder starts at a key frame.
    data.process(2, &packet(5, 1, 1, DELTA, 0, &[0, 0, 1, 0x41])).unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 1);
    data.process(2, &packet(6, 1, 1, KEY, 0, &[0, 0, 1, 0x67])).unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 2);
}

#[test]
fn late_pictures_ask_the_server_for_fewer_frames() {
    let harness = Harness::new();
    let mut data = VideoDataClient::new(Arc::clone(&harness.presenter));
    assert!(harness.presenter().start(&small_request()));

    // Pictures nobody presents overflow the queue; each overflow drops a late picture.
    data.process(2, &packet(1, 1, 1, KEY, 0, &[0, 0, 1, 0x67])).unwrap();
    for sample in 2..12 {
        data.process(
            2,
            &packet(sample, 1, 1, DELTA, u64::from(sample) * 333_333, &[0, 0, 1, 0x41]),
        )
        .unwrap();
    }
    assert!(harness.presenter().take_notifications().is_empty());
    data.process(2, &packet(12, 1, 1, DELTA, 12 * 333_333, &[0, 0, 1, 0x41]))
        .unwrap();
    assert_eq!(
        harness.presenter().take_notifications(),
        [ClientNotification {
            presentation_id: 3,
            notification: Notification::FrameRateOverride {
                flags: Notification::OVERRIDE_FRAME_RATE,
                desired_frame_rate: 22,
            },
        }]
    );

    // Within the same second, more late pictures do not repeat the request.
    for sample in 13..20 {
        data.process(
            2,
            &packet(sample, 1, 1, DELTA, u64::from(sample) * 333_333, &[0, 0, 1, 0x41]),
        )
        .unwrap();
    }
    assert!(harness.presenter().take_notifications().is_empty());
}

#[test]
fn stop_presentation_ignores_its_unused_extra_data_length() {
    let mut stop = bytes(STOP_FIXTURE);
    // cbExtra is at bit offset 480 (MS-RDPEVOR 2.2.1.2) and only header fields are valid for Stop.
    stop[60..64].copy_from_slice(&7u32.to_le_bytes());
    let request = decode::<PresentationRequest>(&stop).unwrap();
    assert_eq!(request.command, PresentationCommand::STOP);
    assert_eq!(request.presentation_id, 3);
    assert!(request.extra_data.is_empty());
}

#[test]
fn a_missing_sample_waits_for_a_key_frame_and_retries_the_request() {
    let harness = Harness::new();
    let mut data = VideoDataClient::new(Arc::clone(&harness.presenter));
    assert!(harness.presenter().start(&small_request()));
    data.process(2, &packet(1, 1, 1, KEY, 0, &[0, 0, 1, 0x67])).unwrap();
    // A repeated sample is ignored.
    data.process(2, &packet(1, 1, 1, KEY, 0, &[0, 0, 1, 0x67])).unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 1);
    assert!(harness.presenter().take_notifications().is_empty());

    // Sample 2 never arrives: sample 3 is not decoded and a key frame is requested.
    data.process(2, &packet(3, 1, 1, DELTA, 666_666, &[0, 0, 1, 0x41]))
        .unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 1);
    let network_error = ClientNotification {
        presentation_id: 3,
        notification: Notification::NetworkError,
    };
    assert_eq!(harness.presenter().take_notifications(), [network_error]);

    // While waiting, the rate-limited request is repeated once the interval has passed.
    data.process(2, &packet(4, 1, 1, DELTA, 999_999, &[0, 0, 1, 0x41]))
        .unwrap();
    assert!(harness.presenter().take_notifications().is_empty());
    harness.advance(10_000_000);
    data.process(2, &packet(5, 1, 1, DELTA, 1_333_332, &[0, 0, 1, 0x41]))
        .unwrap();
    assert_eq!(harness.presenter().take_notifications(), [network_error]);

    data.process(2, &packet(6, 1, 1, KEY, 1_666_665, &[0, 0, 1, 0x67]))
        .unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 2);
}

#[test]
fn the_current_picture_follows_geometry_changes() {
    let harness = Harness::new();
    let mut data = VideoDataClient::new(Arc::clone(&harness.presenter));
    assert!(harness.presenter().start(&small_request()));

    // The picture arrives before its geometry and is shown once the geometry is known.
    data.process(2, &packet(1, 1, 1, KEY, 0, &[0, 0, 1, 0x67])).unwrap();
    assert!(harness.presenter().poll(1024, 768).is_empty());
    harness.presenter().set_geometry(MAPPING_ID, Some(small_geometry()));
    let shown = harness.presenter().poll(1024, 768);
    assert!(!shown.is_empty());
    assert!(harness.presenter().poll(1024, 768).is_empty());

    // Moving the paused video composes the same picture at the new place.
    let mut moved = small_geometry();
    moved.top_level_rect.left += 100;
    harness.presenter().set_geometry(MAPPING_ID, Some(moved));
    let recomposed = harness.presenter().poll(1024, 768);
    assert_eq!(recomposed.len(), shown.len());
    assert_eq!(recomposed[0].region.left, shown[0].region.left + 100);
    assert_eq!(recomposed[0].data, shown[0].data);
    assert_eq!(harness.presenter().statistics(), (1, 0));
}

#[test]
fn resynchronizing_the_clock_moves_queued_pictures_too() {
    let harness = Harness::new();
    let mut data = VideoDataClient::new(Arc::clone(&harness.presenter));
    assert!(harness.presenter().start(&small_request()));
    harness.presenter().set_geometry(MAPPING_ID, Some(small_geometry()));

    // Three pictures 1.5 s apart arrive at once; the last is too far ahead and resynchronizes.
    data.process(2, &packet(1, 1, 1, KEY, 0, &[0, 0, 1, 0x67])).unwrap();
    data.process(2, &packet(2, 1, 1, DELTA, 15_000_000, &[0, 0, 1, 0x41]))
        .unwrap();
    data.process(2, &packet(3, 1, 1, DELTA, 30_000_000, &[0, 0, 1, 0x41]))
        .unwrap();
    assert_eq!(harness.decoded.load(Ordering::SeqCst), 3);

    // The latest picture shows now instead of waiting behind the older queued one.
    assert!(!harness.presenter().poll(1024, 768).is_empty());
    assert_eq!(harness.presenter().statistics(), (1, 2));
    assert_eq!(harness.presenter().next_deadline(), None);
}
