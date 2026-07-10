use std::borrow::Cow;

use ironrdp_core::encode_vec;
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::autodetect::{AutoDetectReqPdu, AutoDetectRequest, AutoDetectResponse, AutoDetectRspPdu};
use ironrdp_pdu::x224::X224;
use ironrdp_session::x224::Processor;
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

    let outputs = processor.process(&frame).unwrap();

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

    let outputs = processor.process(&frame).unwrap();

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

    let outputs = processor.process(&frame).unwrap();

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

    let outputs = processor.process(&frame).unwrap();
    assert!(outputs.is_empty(), "BW start should produce no output");
}

#[test]
fn bandwidth_measure_stop_does_not_crash() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::bw_stop_continuous(200);
    let frame = encode_server_autodetect(request);

    let outputs = processor.process(&frame).unwrap();
    assert!(outputs.is_empty(), "BW stop should produce no output");
}

#[test]
fn bandwidth_measure_payload_does_not_crash() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::bw_payload(300, vec![0xAA; 64]);
    let frame = encode_server_autodetect(request);

    let outputs = processor.process(&frame).unwrap();
    assert!(outputs.is_empty(), "BW payload should produce no output");
}
