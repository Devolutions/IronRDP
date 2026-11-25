//! Bandwidth measurements bracketing a large EGFX write, on the main
//! connection and on the UDP tunnel ([MS-RDPBCGR] 1.3.9).
//!
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/

use ironrdp_core::{ReadCursor, decode, decode_cursor};
use ironrdp_pdu::mcs::SendDataIndication;
use ironrdp_pdu::rdp::autodetect::{AutoDetectReqPdu, AutoDetectRequest};
use ironrdp_pdu::x224::X224;
use ironrdp_rdpemt::{SubHeaderType, TunnelSubHeader};
use ironrdp_server::{ConnectionOutput, ConnectionState, PduContext};
use ironrdp_svc::SvcMessage;

/// The smallest graphics write the server brackets with a bandwidth measurement, 10 KiB.
/// Mirrors a tuning constant that is not public, so retuning it must update this test.
const BW_BRACKET_MIN_BYTES: usize = 10 * 1024;

const IO_CHANNEL_ID: u16 = 1003;
const DRDYNVC_CHANNEL_ID: u16 = 1004;
const USER_CHANNEL_ID: u16 = 1007;
const MESSAGE_CHANNEL_ID: u16 = 1008;

fn context(udp_available: bool) -> PduContext {
    PduContext::new(
        IO_CHANNEL_ID,
        USER_CHANNEL_ID,
        Some(MESSAGE_CHANNEL_ID),
        udp_available,
        0,
    )
}

/// Decodes an auto-detect request sent on the MCS message channel.
fn message_channel_request(frame: &[u8]) -> AutoDetectRequest {
    let indication = decode::<X224<SendDataIndication<'_>>>(frame)
        .expect("an MCS Send Data Indication")
        .0;
    assert_eq!(indication.initiator_id, USER_CHANNEL_ID);
    assert_eq!(indication.channel_id, MESSAGE_CHANNEL_ID);
    decode::<AutoDetectReqPdu>(&indication.user_data)
        .expect("an auto-detect request")
        .request
}

/// Reassembles the static channel data the chunks in `frame` carry on `channel_id`.
fn static_channel_data(frame: &[u8], channel_id: u16) -> Vec<u8> {
    let mut cursor = ReadCursor::new(frame);
    let mut data = Vec::new();
    while !cursor.is_empty() {
        let indication = decode_cursor::<X224<SendDataIndication<'_>>>(&mut cursor)
            .expect("an MCS Send Data Indication")
            .0;
        assert_eq!(indication.initiator_id, USER_CHANNEL_ID);
        assert_eq!(indication.channel_id, channel_id);
        // CHANNEL_PDU_HEADER: length (4 bytes), flags (4 bytes), then the chunk.
        data.extend_from_slice(&indication.user_data[8..]);
    }
    data
}

// oracle: spec MS-RDPBCGR §1.3.9 carries a tunnel bandwidth Start and Stop in RDP_TUNNEL_SUBHEADERs, and the client counts the bytes between them.
// oracle: mutant putting a bandwidth Start or Stop in a Tunnel Data PDU with graphics data, or moving either across the graphics.
#[test]
fn a_measured_udp_graphics_batch_is_bracketed_by_bare_tunnel_pdus() {
    let mut conn = ConnectionState::with_autodetect();
    // Together large enough to be worth measuring.
    let first = vec![1; BW_BRACKET_MIN_BYTES / 2];
    let second = vec![2; BW_BRACKET_MIN_BYTES / 2];
    let messages = vec![SvcMessage::from(first.clone()), SvcMessage::from(second.clone())];

    let outputs = conn
        .encode_egfx_over_udp(&context(true), messages)
        .expect("encode the batch");

    let bare = |output: &ConnectionOutput| match output {
        ConnectionOutput::TunnelData { sub_headers, data } if data.is_empty() => match sub_headers.as_slice() {
            [sub_header] => Some(sub_header.clone()),
            _ => None,
        },
        _ => None,
    };
    let payload = |output: &ConnectionOutput| match output {
        ConnectionOutput::TunnelData { sub_headers, data } if sub_headers.is_empty() => Some(data.clone()),
        _ => None,
    };
    // RDP_BW_START and RDP_BW_STOP as SubHeaderData: sequenceNumber, then requestType 0x0014 and
    // 0x0429 for reliable UDP (MS-RDPBCGR 2.2.14.1.2 and 2.2.14.1.4).
    let tunnel_request = |sequence: [u8; 2], request_type: [u8; 2]| TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectRequest,
        data: vec![sequence[0], sequence[1], request_type[0], request_type[1]],
    };
    assert_eq!(outputs.len(), 4);
    let start = bare(&outputs[0]).expect("Start alone in the first Tunnel Data PDU");
    let sequence = [start.data[0], start.data[1]];
    assert_eq!(start, tunnel_request(sequence, [0x14, 0x00]));
    assert_eq!(payload(&outputs[1]), Some(first));
    assert_eq!(payload(&outputs[2]), Some(second));
    assert_eq!(bare(&outputs[3]), Some(tunnel_request(sequence, [0x29, 0x04])));
}

// oracle: spec MS-RDPBCGR §1.3.9 and §2.2.14.3 bracket the measured data with a Start and a Stop on the MCS message channel.
// oracle: mutant moving the bandwidth Start or Stop across the graphics on TCP, or off the message channel.
#[test]
fn a_measured_tcp_graphics_batch_is_bracketed_on_the_message_channel() {
    let mut conn = ConnectionState::with_autodetect();
    // Large enough to be worth measuring.
    let graphics = vec![1; BW_BRACKET_MIN_BYTES];

    let outputs = conn
        .encode_egfx_over_tcp(
            &context(false),
            DRDYNVC_CHANNEL_ID,
            vec![SvcMessage::from(graphics.clone())],
        )
        .expect("encode the batch");

    let frames: Vec<&[u8]> = outputs
        .iter()
        .map(|output| match output {
            ConnectionOutput::ResponseFrame(frame) => frame.as_slice(),
            other => panic!("TCP graphics produced {other:?}"),
        })
        .collect();
    assert_eq!(frames.len(), 3);
    let start = message_channel_request(frames[0]);
    let sequence = start.sequence_number();
    assert_eq!(start, AutoDetectRequest::bw_start_continuous(sequence));
    assert_eq!(static_channel_data(frames[1], DRDYNVC_CHANNEL_ID), graphics);
    assert_eq!(
        message_channel_request(frames[2]),
        AutoDetectRequest::bw_stop_continuous(sequence)
    );
}
