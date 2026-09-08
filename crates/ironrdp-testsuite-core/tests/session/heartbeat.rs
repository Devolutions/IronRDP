use std::borrow::Cow;

use ironrdp_core::encode_vec;
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::autodetect::{AutoDetectReqPdu, AutoDetectRequest};
use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use ironrdp_pdu::rdp::heartbeat::HeartbeatPdu;
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

fn process_frame(processor: &mut Processor, frame: &[u8]) -> Vec<ironrdp_session::x224::ProcessorOutput> {
    let mut bulk_decompressor = None;
    processor.process(frame, &mut bulk_decompressor).expect("process frame")
}

/// Encode a Heartbeat PDU as a server-to-client SendDataIndication on the MCS
/// message channel ([MS-RDPBCGR] 2.2.16.1): framed by a Basic Security Header
/// (SEC_HEARTBEAT), the same channel Auto-Detect traffic shares.
fn encode_server_heartbeat(period: u8, count1: u8, count2: u8) -> Vec<u8> {
    let pdu = HeartbeatPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::HEARTBEAT,
        },
        period,
        count1,
        count2,
    };
    let user_data = encode_vec(&pdu).unwrap();

    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: MESSAGE_CHANNEL_ID,
        user_data: Cow::Owned(user_data),
    });

    encode_vec(&X224(indication)).unwrap()
}

/// A Heartbeat PDU is purely informational: [MS-RDPBCGR] 2.2.16.1 defines no
/// client response, unlike Auto-Detect RTT requests on the same channel.
#[test]
fn heartbeat_produces_no_response() {
    let mut processor = make_processor();
    let frame = encode_server_heartbeat(30, 4, 8);

    let outputs = process_frame(&mut processor, &frame);

    assert!(outputs.is_empty(), "Heartbeat PDU must not produce any output");
}

/// The message channel demux must still route ordinary Auto-Detect traffic
/// correctly once it also recognizes Heartbeat: the two PDU families are told
/// apart by security-header flags before either decode is attempted.
#[test]
fn autodetect_still_works_alongside_heartbeat() {
    let mut processor = make_processor();

    let heartbeat_frame = encode_server_heartbeat(30, 4, 8);
    assert!(process_frame(&mut processor, &heartbeat_frame).is_empty());

    let autodetect_pdu = AutoDetectReqPdu::new(AutoDetectRequest::rtt_continuous(7));
    let user_data = encode_vec(&autodetect_pdu).unwrap();
    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: MESSAGE_CHANNEL_ID,
        user_data: Cow::Owned(user_data),
    });
    let autodetect_frame = encode_vec(&X224(indication)).unwrap();

    let outputs = process_frame(&mut processor, &autodetect_frame);
    assert_eq!(outputs.len(), 1, "RTT request must still produce a response frame");
}

/// A message-channel PDU with a flag combination this session doesn't
/// recognize must be ignored, not treated as a session-fatal decode error:
/// the channel is forward-safe for future MS-RDPBCGR message-channel PDU
/// types the same way the connect-time demux already is.
#[test]
fn unrecognized_message_channel_pdu_is_ignored_not_fatal() {
    let mut processor = make_processor();

    // A bare BasicSecurityHeader with no recognized discriminator flag, no
    // trailing payload.
    let unrecognized = BasicSecurityHeader {
        flags: BasicSecurityHeaderFlags::empty(),
    };
    let user_data = encode_vec(&unrecognized).unwrap();
    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: MESSAGE_CHANNEL_ID,
        user_data: Cow::Owned(user_data),
    });
    let frame = encode_vec(&X224(indication)).unwrap();

    let outputs = process_frame(&mut processor, &frame);
    assert!(outputs.is_empty(), "unrecognized PDU must be ignored, not error");
}
