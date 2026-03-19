use std::borrow::Cow;

use ironrdp_connector::connection_activation::ConnectionActivationSequence;
use ironrdp_connector::{Credentials, DesktopSize};
use ironrdp_core::encode_vec;
use ironrdp_pdu::gcc;
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::client_info::CompressionType;
use ironrdp_pdu::rdp::headers::{
    CompressionFlags, ShareControlHeader, ShareControlPdu, ShareDataHeader, ShareDataPdu, StreamPriority,
};
use ironrdp_pdu::x224::X224;
use ironrdp_session::x224::Processor;
use ironrdp_svc::StaticChannelSet;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;
const SHARE_ID: u32 = 0x0001_0000;

fn test_config() -> ironrdp_connector::Config {
    ironrdp_connector::Config {
        desktop_size: DesktopSize {
            width: 1024,
            height: 768,
        },
        desktop_scale_factor: 0,
        enable_tls: true,
        enable_credssp: false,
        credentials: Credentials::UsernamePassword {
            username: "test".into(),
            password: "test".into(),
        },
        domain: None,
        client_build: 0,
        client_name: "test".into(),
        keyboard_type: gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        license_cache: None,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
        multitransport_flags: None,
        performance_flags: Default::default(),
        timezone_info: Default::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

fn make_processor() -> Processor {
    let config = test_config();
    let cas = ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID);
    Processor::new(StaticChannelSet::new(), USER_CHANNEL_ID, IO_CHANNEL_ID, SHARE_ID, cas)
}

/// Encode a ShareDataPdu as a server-to-client SendDataIndication frame.
fn encode_server_share_data(pdu: ShareDataPdu) -> Vec<u8> {
    let share_data_header = ShareDataHeader {
        share_data_pdu: pdu,
        stream_priority: StreamPriority::Medium,
        compression_flags: CompressionFlags::empty(),
        compression_type: CompressionType::K8,
    };

    let share_control_header = ShareControlHeader {
        share_control_pdu: ShareControlPdu::Data(share_data_header),
        pdu_source: USER_CHANNEL_ID,
        share_id: SHARE_ID,
    };

    let user_data = encode_vec(&share_control_header).unwrap();

    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: IO_CHANNEL_ID,
        user_data: Cow::Owned(user_data),
    });

    encode_vec(&X224(indication)).unwrap()
}

#[test]
fn rtt_request_produces_response_frame() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::rtt_continuous(42);
    let frame = encode_server_share_data(ShareDataPdu::AutoDetectReq(request));

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
    let frame = encode_server_share_data(ShareDataPdu::AutoDetectReq(request));

    let outputs = processor.process(&frame).unwrap();

    assert_eq!(outputs.len(), 1);
    let ironrdp_session::x224::ProcessorOutput::ResponseFrame(response_data) = &outputs[0] else {
        panic!("expected ResponseFrame");
    };

    // The response frame wraps X224 > MCS SendDataRequest > ShareControl > ShareData > AutoDetectRsp.
    // Decode the MCS layer to extract user data, then decode the share headers.
    let mcs_msg = ironrdp_core::decode::<X224<McsMessage<'_>>>(response_data).unwrap();
    let McsMessage::SendDataRequest(send_data) = mcs_msg.0 else {
        panic!("expected SendDataRequest in response frame");
    };
    let share_control = ironrdp_core::decode::<ShareControlHeader>(&send_data.user_data).unwrap();
    let ShareControlPdu::Data(share_data) = share_control.share_control_pdu else {
        panic!("expected Data PDU in ShareControl");
    };

    match share_data.share_data_pdu {
        ShareDataPdu::AutoDetectRsp(AutoDetectResponse::RttResponse {
            sequence_number: rsp_seq,
        }) => {
            assert_eq!(rsp_seq, sequence_number, "sequence number must be echoed");
        }
        other => panic!("expected AutoDetectRsp(RttResponse), got {other:?}"),
    }
}

#[test]
fn network_characteristics_result_surfaces_as_autodetect() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::netchar_result(7, 10, 50000, 20);
    let frame = encode_server_share_data(ShareDataPdu::AutoDetectReq(request.clone()));

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
    let frame = encode_server_share_data(ShareDataPdu::AutoDetectReq(request));

    let outputs = processor.process(&frame).unwrap();
    assert!(outputs.is_empty(), "BW start should produce no output");
}

#[test]
fn bandwidth_measure_stop_does_not_crash() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::bw_stop_continuous(200);
    let frame = encode_server_share_data(ShareDataPdu::AutoDetectReq(request));

    let outputs = processor.process(&frame).unwrap();
    assert!(outputs.is_empty(), "BW stop should produce no output");
}

#[test]
fn bandwidth_measure_payload_does_not_crash() {
    let mut processor = make_processor();
    let request = AutoDetectRequest::bw_payload(300, vec![0xAA; 64]);
    let frame = encode_server_share_data(ShareDataPdu::AutoDetectReq(request));

    let outputs = processor.process(&frame).unwrap();
    assert!(outputs.is_empty(), "BW payload should produce no output");
}
