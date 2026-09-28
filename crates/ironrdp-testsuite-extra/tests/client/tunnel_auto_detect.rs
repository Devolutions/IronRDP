//! Auto-detect requests and responses travel on the UDP tunnel as tunnel sub-headers, whose
//! own two bytes are the auto-detect structure's headerLength and headerTypeId ([MS-RDPEMT]
//! 2.2.1.1.1).
//!
//! [MS-RDPEMT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/4f538fd7-3aca-4e7d-a213-13eb5f95c1ad

use ironrdp_client::rdp::{tunnel_auto_detect_requests, tunnel_auto_detect_sub_header};
use ironrdp_core::encode_vec;
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};
use ironrdp_rdpemt::{SubHeaderType, TunnelData, TunnelSubHeader};

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

    let requests = tunnel_auto_detect_requests(&[rtt, unmodeled, response]);
    assert!(matches!(
        requests.as_slice(),
        [AutoDetectRequest::RttRequest { sequence_number: 7, .. }]
    ));
}

/// The RTT response goes back as `06 01 <seq> 00 00`, alone in a Tunnel Data PDU with no data.
#[test]
fn auto_detect_responses_encode_as_tunnel_sub_headers() {
    let response = AutoDetectResponse::RttResponse { sequence_number: 7 };
    let sub_header = tunnel_auto_detect_sub_header(&response).expect("sub-header");
    let pdu = encode_vec(&TunnelData {
        sub_headers: vec![sub_header],
        higher_layer_data: Vec::new(),
    })
    .expect("encode");
    // TunnelData: action 2, payloadLength 0, headerLength 4 + 6, then the sub-header.
    assert_eq!(pdu, [0x02, 0x00, 0x00, 0x0A, 0x06, 0x01, 0x07, 0x00, 0x00, 0x00]);
}
