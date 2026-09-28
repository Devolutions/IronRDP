use std::borrow::Cow;

use ironrdp_core::encode_vec;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::Action;
use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::autodetect::{AutoDetectReqPdu, AutoDetectRequest, AutoDetectResponse, AutoDetectRspPdu};
use ironrdp_pdu::x224::X224;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::x224::Processor;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_svc::StaticChannelSet;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;
const MESSAGE_CHANNEL_ID: u16 = 1004;
const SHARE_ID: u32 = 0x0001_0000;

fn make_processor() -> Processor {
    Processor::new(
        StaticChannelSet::new(),
        USER_CHANNEL_ID,
        IO_CHANNEL_ID,
        Some(MESSAGE_CHANNEL_ID),
        SHARE_ID,
    )
}

fn process_frame(processor: &mut Processor, frame: &[u8]) -> Vec<ironrdp_session::x224::ProcessorOutput> {
    let mut bulk_decompressor = None;
    processor.process(frame, &mut bulk_decompressor).expect("process frame")
}

/// Encode an Auto-Detect Request as a server-to-client SendDataIndication on the
/// MCS message channel ([MS-RDPBCGR] 2.2.14.3): the auto-detect data is framed by
/// a Basic Security Header (SEC_AUTODETECT_REQ), not a Share Data header.
fn encode_server_autodetect(request: AutoDetectRequest) -> Vec<u8> {
    let pdu = AutoDetectReqPdu::new(request);
    let user_data = encode_vec(&pdu).unwrap();

    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: MESSAGE_CHANNEL_ID,
        user_data: Cow::Owned(user_data),
    });

    encode_vec(&X224(indication)).unwrap()
}

#[test]
fn rtt_request_produces_response_frame() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::rtt_continuous(42);
    let frame = encode_server_autodetect(request);

    let outputs = process_frame(&mut processor, &frame);

    assert_eq!(outputs.len(), 1);
    match &outputs[0] {
        ironrdp_session::x224::ProcessorOutput::ResponseFrame(data) => {
            assert!(!data.is_empty(), "response frame must not be empty");
        }
        other => panic!("expected ResponseFrame, got {other:?}"),
    }
}

#[test]
fn rtt_response_preserves_sequence_number() {
    let mut processor = make_processor();
    let sequence_number = 0x1234;
    let request = AutoDetectRequest::rtt_connect_time(sequence_number);
    let frame = encode_server_autodetect(request);

    let outputs = process_frame(&mut processor, &frame);

    assert_eq!(outputs.len(), 1);
    let ironrdp_session::x224::ProcessorOutput::ResponseFrame(response_data) = &outputs[0] else {
        panic!("expected ResponseFrame");
    };

    // The response is a Client Auto-Detect Response PDU on the message channel:
    // X224 > MCS SendDataRequest > BasicSecurityHeader(SEC_AUTODETECT_RSP) > data.
    let mcs_msg = ironrdp_core::decode::<X224<McsMessage<'_>>>(response_data).unwrap();
    let McsMessage::SendDataRequest(send_data) = mcs_msg.0 else {
        panic!("expected SendDataRequest in response frame");
    };
    assert_eq!(
        send_data.channel_id, MESSAGE_CHANNEL_ID,
        "response must be sent on the message channel"
    );

    let response = ironrdp_core::decode::<AutoDetectRspPdu>(&send_data.user_data).unwrap();
    match response.response {
        AutoDetectResponse::RttResponse {
            sequence_number: rsp_seq,
        } => {
            assert_eq!(rsp_seq, sequence_number, "sequence number must be echoed");
        }
        other => panic!("expected RttResponse, got {other:?}"),
    }
}

#[test]
fn network_characteristics_result_surfaces_as_autodetect() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::netchar_result(7, 10, 50000, 20);
    let frame = encode_server_autodetect(request.clone());

    let outputs = process_frame(&mut processor, &frame);

    assert_eq!(outputs.len(), 1);
    match &outputs[0] {
        ironrdp_session::x224::ProcessorOutput::AutoDetect(req) => {
            assert_eq!(req, &request, "surfaced request must match the original");
        }
        other => panic!("expected AutoDetect output, got {other:?}"),
    }
}

#[test]
fn bandwidth_measure_start_does_not_crash() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::bw_start_connect_time(100);
    let frame = encode_server_autodetect(request);

    let outputs = process_frame(&mut processor, &frame);
    assert!(outputs.is_empty(), "BW start should produce no output");
}

#[test]
fn bandwidth_measure_stop_does_not_crash() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::bw_stop_continuous(200);
    let frame = encode_server_autodetect(request);

    let outputs = process_frame(&mut processor, &frame);
    assert_eq!(bandwidth_result(&outputs), (200, 0x000b, 1, 0));
}

#[test]
fn bandwidth_measure_payload_does_not_crash() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::bw_payload(300, vec![0xAA; 64]);
    let frame = encode_server_autodetect(request);

    let outputs = process_frame(&mut processor, &frame);
    assert!(outputs.is_empty(), "BW payload should produce no output");
}

fn timed_request(
    processor: &mut Processor,
    request: AutoDetectRequest,
    millis: u64,
) -> Vec<ironrdp_session::x224::ProcessorOutput> {
    processor
        .process_with_timestamp(
            &encode_server_autodetect(request),
            &mut None,
            Some(ironrdp_core::MonotonicInstant::from_millis(millis)),
        )
        .expect("process timed auto-detect request")
}

fn bandwidth_result(outputs: &[ironrdp_session::x224::ProcessorOutput]) -> (u16, u16, u32, u32) {
    let [ironrdp_session::x224::ProcessorOutput::ResponseFrame(frame)] = outputs else {
        panic!("expected exactly one bandwidth response");
    };
    bandwidth_result_frame(frame)
}

fn bandwidth_result_frame(frame: &[u8]) -> (u16, u16, u32, u32) {
    let X224(McsMessage::SendDataRequest(message)) = ironrdp_core::decode::<X224<McsMessage<'_>>>(frame).unwrap()
    else {
        panic!("expected main-channel response");
    };
    assert_eq!(message.channel_id, MESSAGE_CHANNEL_ID);
    let response = ironrdp_core::decode::<AutoDetectRspPdu>(&message.user_data).unwrap();
    let AutoDetectResponse::BandwidthMeasureResults {
        sequence_number,
        response_type,
        time_delta_ms,
        byte_count,
    } = response.response
    else {
        panic!("expected bandwidth results");
    };
    (sequence_number, response_type, time_delta_ms, byte_count)
}

#[test]
fn continuous_measurement_counts_data_without_security_headers() {
    let mut processor = make_processor();
    assert!(timed_request(&mut processor, AutoDetectRequest::bw_start_continuous(1), 10).is_empty());
    timed_request(&mut processor, AutoDetectRequest::rtt_continuous(2), 20);
    let result = timed_request(&mut processor, AutoDetectRequest::bw_stop_continuous(3), 35);
    // RTT and Stop have six-byte auto-detect headers. Neither four-byte
    // security header, nor TPKT/X224/MCS framing, belongs to the count.
    assert_eq!(bandwidth_result(&result), (3, 0x000b, 25, 12));
}

#[test]
fn connect_time_measurement_includes_payload_headers_once() {
    let mut processor = make_processor();
    timed_request(&mut processor, AutoDetectRequest::bw_start_connect_time(1), 100);
    timed_request(&mut processor, AutoDetectRequest::bw_payload(2, vec![0xaa; 64]), 110);
    timed_request(&mut processor, AutoDetectRequest::rtt_continuous(3), 112);
    let result = timed_request(
        &mut processor,
        AutoDetectRequest::bw_stop_connect_time(4, vec![0xbb; 16]),
        140,
    );
    assert_eq!(bandwidth_result(&result), (4, 0x0003, 40, 64 + 8 + 16 + 8));
}

#[test]
fn repeated_start_resets_count_and_timer_and_stop_ends_window() {
    let mut processor = make_processor();
    timed_request(&mut processor, AutoDetectRequest::bw_start_continuous(1), 100);
    timed_request(&mut processor, AutoDetectRequest::rtt_continuous(2), 110);
    timed_request(&mut processor, AutoDetectRequest::bw_start_continuous(3), 200);
    let result = timed_request(&mut processor, AutoDetectRequest::bw_stop_continuous(4), 210);
    assert_eq!(bandwidth_result(&result), (4, 0x000b, 10, 6));
    let repeated = timed_request(&mut processor, AutoDetectRequest::bw_stop_continuous(5), 220);
    assert_eq!(bandwidth_result(&repeated), (5, 0x000b, 1, 0));
}

#[test]
fn measurement_timing_saturates_and_never_reports_zero_divisor() {
    for (start, stop, expected) in [(100, 100, 1), (100, 90, 1), (0, u64::MAX, u32::MAX)] {
        let mut processor = make_processor();
        timed_request(&mut processor, AutoDetectRequest::bw_start_continuous(1), start);
        let result = timed_request(&mut processor, AutoDetectRequest::bw_stop_continuous(2), stop);
        assert_eq!(bandwidth_result(&result), (2, 0x000b, expected, 6));
    }
}

#[test]
fn lossy_requests_on_main_channel_are_not_answered() {
    use ironrdp_pdu::rdp::autodetect::{BW_START_LOSSY_UDP, BW_STOP_LOSSY_UDP};
    let mut processor = make_processor();
    assert!(
        timed_request(
            &mut processor,
            AutoDetectRequest::BandwidthMeasureStart {
                sequence_number: 1,
                request_type: BW_START_LOSSY_UDP,
            },
            10
        )
        .is_empty()
    );
    assert!(
        timed_request(
            &mut processor,
            AutoDetectRequest::BandwidthMeasureStop {
                sequence_number: 1,
                request_type: BW_STOP_LOSSY_UDP,
                payload: None,
            },
            20
        )
        .is_empty()
    );
}

#[test]
fn untimed_driver_does_not_report_accumulated_bytes_as_a_real_measurement() {
    let mut processor = make_processor();
    timed_request(&mut processor, AutoDetectRequest::bw_start_continuous(1), 10);
    timed_request(&mut processor, AutoDetectRequest::rtt_continuous(2), 20);
    let outputs = process_frame(
        &mut processor,
        &encode_server_autodetect(AutoDetectRequest::bw_stop_continuous(3)),
    );
    assert_eq!(bandwidth_result(&outputs), (3, 0x000b, 1, 0));
}

fn make_active_stage() -> ActiveStage {
    ActiveStageBuilder {
        static_channels: StaticChannelSet::new(),
        user_channel_id: USER_CHANNEL_ID,
        io_channel_id: IO_CHANNEL_ID,
        message_channel_id: Some(MESSAGE_CHANNEL_ID),
        share_id: SHARE_ID,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build()
}

/// A fast-path frame carrying one Synchronize update with `data_len` bytes of data.
fn fast_path_frame(data_len: usize) -> Vec<u8> {
    let data = vec![0; data_len];
    let update = encode_vec(&FastPathUpdatePdu {
        fragmentation: Fragmentation::Single,
        update_code: UpdateCode::Synchronize,
        compression_flags: None,
        compression_type: None,
        data: &data,
    })
    .unwrap();
    let mut frame = encode_vec(&FastPathHeader::new(EncryptionFlags::empty(), update.len())).unwrap();
    frame.extend_from_slice(&update);
    frame
}

fn process_stage_frame(
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    action: Action,
    frame: &[u8],
    millis: u64,
) -> Vec<ActiveStageOutput> {
    stage
        .process_with_timestamp(
            image,
            action,
            frame,
            Some(ironrdp_core::MonotonicInstant::from_millis(millis)),
        )
        .expect("process timed frame")
}

#[test]
fn continuous_measurement_counts_fast_path_data_only_inside_the_window() {
    let mut stage = make_active_stage();
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);
    let frame = fast_path_frame(100);

    process_stage_frame(&mut stage, &mut image, Action::FastPath, &frame, 5);
    let start = encode_server_autodetect(AutoDetectRequest::bw_start_continuous(1));
    process_stage_frame(&mut stage, &mut image, Action::X224, &start, 10);
    process_stage_frame(&mut stage, &mut image, Action::FastPath, &frame, 20);
    let stop = encode_server_autodetect(AutoDetectRequest::bw_stop_continuous(2));
    let outputs = process_stage_frame(&mut stage, &mut image, Action::X224, &stop, 40);
    process_stage_frame(&mut stage, &mut image, Action::FastPath, &frame, 50);

    let [ActiveStageOutput::ResponseFrame(response)] = outputs.as_slice() else {
        panic!("expected exactly one bandwidth response, got {outputs:?}");
    };
    // The frame between Start and Stop counts without its fast-path header: a
    // three-byte update header and 100 bytes of update data. The Stop adds its
    // six-byte auto-detect header, as in the message-channel tests above.
    assert_eq!(bandwidth_result_frame(response), (2, 0x000b, 30, 3 + 100 + 6));
}
