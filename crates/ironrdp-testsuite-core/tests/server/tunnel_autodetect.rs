//! Continuous Auto-Detection over the UDP tunnel, where Start, Stop and the
//! client's results travel in RDP_TUNNEL_SUBHEADERs ([MS-RDPBCGR] 1.3.9).

use core::net::Ipv4Addr;

use ironrdp_core::encode_vec;
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};
use ironrdp_rdpemt::{SubHeaderType, TunnelSubHeader};
use ironrdp_server::autodetect::BW_BRACKET_MIN_BYTES;
use ironrdp_server::{RdpServer, autodetect_sub_header};

fn server_with_autodetect() -> RdpServer {
    let mut server = RdpServer::builder()
        .with_addr((Ipv4Addr::LOCALHOST, 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build();
    server.enable_autodetect();
    server
}

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

/// A measurement the tunnel took with it when it closed does not hold up
/// the next one, which after the fallback goes over TCP.
#[test]
fn the_tunnel_closing_cancels_its_bandwidth_measurement() {
    let mut server = server_with_autodetect();
    let ad = server.autodetect_mut().expect("auto-detect enabled");
    ad.begin_bandwidth_measure(BW_BRACKET_MIN_BYTES, 0)
        .expect("a large write starts a measurement");
    ad.end_bandwidth_measure().expect("the measurement is open");
    server.mark_bandwidth_measure_on_udp();

    server.cancel_udp_bandwidth_measure();

    let ad = server.autodetect_mut().expect("auto-detect enabled");
    assert!(ad.begin_bandwidth_measure(BW_BRACKET_MIN_BYTES, 1_000).is_some());
}

#[test]
fn bandwidth_results_on_the_tunnel_complete_the_measurement() {
    let mut server = server_with_autodetect();
    let ad = server.autodetect_mut().expect("auto-detect enabled");
    let start = ad
        .begin_bandwidth_measure(BW_BRACKET_MIN_BYTES, 0)
        .expect("a large write starts a measurement");
    ad.end_bandwidth_measure().expect("the measurement is open");

    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: start.sequence_number(),
        response_type: 0x000B,
        time_delta_ms: 1000,
        byte_count: 125_000,
    };
    server.record_tunnel_sub_headers(&[
        // Malformed, dropped without affecting the one after it.
        TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectResponse,
            data: vec![0x01],
        },
        TunnelSubHeader {
            sub_header_type: SubHeaderType::AutoDetectResponse,
            // The sub-header supplies headerLength and headerTypeId itself.
            data: encode_vec(&results).expect("encode Bandwidth Measure Results")[2..].to_vec(),
        },
    ]);

    assert_eq!(server.autodetect_mut().and_then(|ad| ad.bandwidth_kbps()), Some(1000));
}
