//! Auto-detect requests and responses travel on the UDP tunnel as tunnel sub-headers, whose
//! own two bytes are the auto-detect structure's headerLength and headerTypeId ([MS-RDPEMT]
//! 2.2.1.1.1).
//!
//! [MS-RDPEMT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/4f538fd7-3aca-4e7d-a213-13eb5f95c1ad

use ironrdp_core::encode_vec;
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};
use ironrdp_rdpemt::{SubHeaderType, TunnelData, TunnelSubHeader};
use ironrdp_rdpeudp_tokio::TunnelMessage;

fn message(sub_headers: Vec<TunnelSubHeader>) -> TunnelMessage {
    TunnelMessage {
        sub_headers,
        data: Vec::new(),
    }
}

/// Windows frames an RTT Measure Request on the tunnel as `06 00 <seq> 01 00`. Request types the
/// decoder does not model, and sub-headers that are not requests, are skipped.
#[test]
fn tunnel_sub_headers_decode_as_auto_detect_requests() {
    let rtt: TunnelSubHeader = ironrdp_core::decode(&[0x06, 0x00, 0x07, 0x00, 0x01, 0x00]).expect("sub-header");
    let unmodeled = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectRequest,
        data: vec![0x01, 0x00, 0xFF, 0x7F],
    };
    let response = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectResponse,
        data: vec![0x07, 0x00, 0x00, 0x00],
    };

    let requests = message(vec![rtt, unmodeled, response]).auto_detect_requests();
    assert!(matches!(
        requests.as_slice(),
        [AutoDetectRequest::RttRequest { sequence_number: 7, .. }]
    ));
}

/// The RTT response goes back as `06 01 <seq> 00 00`, alone in a Tunnel Data PDU with no data.
#[test]
fn auto_detect_responses_encode_as_tunnel_sub_headers() {
    let reply = TunnelMessage::from_auto_detect_responses(&[AutoDetectResponse::RttResponse { sequence_number: 7 }])
        .expect("reply");
    let pdu = encode_vec(&TunnelData {
        sub_headers: reply.sub_headers,
        higher_layer_data: reply.data,
    })
    .expect("encode");
    // TunnelData: action 2, payloadLength 0, headerLength 4 + 6, then the sub-header.
    assert_eq!(pdu, [0x02, 0x00, 0x00, 0x0A, 0x06, 0x01, 0x07, 0x00, 0x00, 0x00]);
}

#[test]
fn no_responses_make_no_reply() {
    assert_eq!(TunnelMessage::from_auto_detect_responses(&[]), None);
}

#[test]
fn bandwidth_sub_headers_preserve_the_wire_layout() {
    let start = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectRequest,
        data: vec![9, 0, 0x14, 0],
    };
    assert_eq!(
        message(vec![start]).auto_detect_requests(),
        [AutoDetectRequest::bw_start_continuous(9)]
    );
    let response = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: 9,
        response_type: 0x000b,
        time_delta_ms: 40,
        byte_count: 150,
    };
    let reply = TunnelMessage::from_auto_detect_responses(&[response]).expect("bandwidth reply");
    let [sub_header] = reply.sub_headers.as_slice() else {
        panic!("expected one sub-header, got {:?}", reply.sub_headers);
    };
    assert_eq!(
        encode_vec(sub_header).unwrap(),
        [14, 1, 9, 0, 11, 0, 40, 0, 0, 0, 150, 0, 0, 0]
    );
}
