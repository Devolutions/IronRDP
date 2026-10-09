//! Continuous Auto-Detection over the UDP tunnel, where Start, Stop and the
//! client's results travel in RDP_TUNNEL_SUBHEADERs ([MS-RDPBCGR] 1.3.9).

use ironrdp_core::encode_vec;
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};
use ironrdp_rdpemt::{SubHeaderType, TunnelSubHeader};
use ironrdp_server::autodetect::AutoDetectManager;
use ironrdp_server::{autodetect_response_from_sub_header, autodetect_sub_header};

/// On the tunnel the request is the sub-header itself, in a PDU with no
/// data, so the client's byte count covers only the payloads sent between
/// Start and Stop.
#[test]
fn a_tunnel_auto_detect_request_is_bare_and_alone() {
    let message =
        autodetect_sub_header(&AutoDetectRequest::bw_start_continuous(7)).expect("encode Bandwidth Measure Start");

    assert!(message.data.is_empty());
    assert_eq!(message.sub_headers.len(), 1);
    assert_eq!(message.sub_headers[0].sub_header_type, SubHeaderType::AutoDetectRequest);
    // RDP_BW_START is the sub-header: SubHeaderLength/headerLength 6,
    // SubHeaderType/headerTypeId TYPE_ID_AUTODETECT_REQUEST, then
    // sequenceNumber 7 and requestType 0x0014 as SubHeaderData.
    assert_eq!(message.sub_headers[0].data, [0x07, 0x00, 0x14, 0x00]);
    assert_eq!(
        encode_vec(&message.sub_headers[0]).expect("encode the sub-header"),
        [0x06, 0x00, 0x07, 0x00, 0x14, 0x00]
    );
}

#[test]
fn bandwidth_results_on_the_tunnel_complete_the_measurement() {
    let mut mgr = AutoDetectManager::new();
    let start = mgr
        .begin_bandwidth_measure(64 * 1024, 0)
        .expect("a large write starts a measurement");
    mgr.end_bandwidth_measure().expect("the measurement is open");

    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: start.sequence_number(),
        response_type: 0x000B,
        time_delta_ms: 1000,
        byte_count: 125_000,
    };
    let sub_headers = [
        // Malformed, dropped without affecting the one after it.
        TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectResponse,
            data: vec![0x01],
        },
        // Not something the client sends on the tunnel.
        TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectRequest,
            data: vec![0x07, 0x00, 0x14, 0x00],
        },
        TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectResponse,
            // The sub-header supplies headerLength and headerTypeId itself.
            data: encode_vec(&results).expect("encode Bandwidth Measure Results")[2..].to_vec(),
        },
    ];

    for sub_header in &sub_headers {
        if let Some(response) = autodetect_response_from_sub_header(sub_header) {
            let _ = mgr.handle_response(&response, 10);
        }
    }

    assert!(autodetect_response_from_sub_header(&sub_headers[0]).is_none());
    assert!(autodetect_response_from_sub_header(&sub_headers[1]).is_none());
    assert_eq!(mgr.bandwidth_kbps(), Some(1000));
}
