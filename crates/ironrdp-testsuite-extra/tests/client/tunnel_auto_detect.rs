//! Auto-detect requests and responses travel on the UDP tunnel as tunnel sub-headers, whose
//! own two bytes are the auto-detect structure's headerLength and headerTypeId ([MS-RDPEMT]
//! 2.2.1.1.1).
//!
//! [MS-RDPEMT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/4f538fd7-3aca-4e7d-a213-13eb5f95c1ad

use ironrdp_client::udp::{tunnel_auto_detect_requests, tunnel_auto_detect_sub_header};
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

#[test]
fn bandwidth_sub_headers_preserve_the_wire_layout() {
    let start = TunnelSubHeader {
        sub_header_type: SubHeaderType::AutoDetectRequest,
        data: vec![9, 0, 0x14, 0],
    };
    assert_eq!(
        tunnel_auto_detect_requests(&[start]),
        [AutoDetectRequest::bw_start_continuous(9)]
    );
    let response = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: 9,
        response_type: 0x000b,
        time_delta_ms: 40,
        byte_count: 150,
    };
    let sub_header = tunnel_auto_detect_sub_header(&response).expect("bandwidth sub-header");
    assert_eq!(
        encode_vec(&sub_header).unwrap(),
        [14, 1, 9, 0, 11, 0, 40, 0, 0, 0, 150, 0, 0, 0]
    );
}

#[test]
fn tunnel_failure_before_migration_withdraws_soft_sync_without_ending_the_session() {
    let mut stage = make_stage(false);
    let mut transport = None;
    ironrdp_client::udp::disable_failed_tunnel(
        &mut stage,
        &mut transport,
        ironrdp::session::general_err!("auto-detect reply send failed"),
    )
    .expect("unused tunnel must fall back to TCP");
    assert!(transport.is_none());
    assert!(!stage.reliable_udp_dvc_tunnel_in_use());

    // The failed tunnel must no longer be offered to a later Soft-Sync request.
    use ironrdp_svc::SvcProcessor as _;
    let drdynvc = stage.get_svc_processor_mut::<ironrdp_dvc::DrdynvcClient>().unwrap();
    assert!(drdynvc.process(&soft_sync_request()).is_err());
}

#[test]
fn tunnel_failure_after_migration_requires_reconnection() {
    let mut stage = make_stage(true);
    let mut transport = None;
    let error = ironrdp_client::udp::disable_failed_tunnel(
        &mut stage,
        &mut transport,
        ironrdp::session::general_err!("auto-detect reply send failed"),
    )
    .expect_err("migrated channels cannot silently resume on TCP");
    assert!(error.to_string().contains("auto-detect reply send failed"));
    assert!(stage.reliable_udp_dvc_tunnel_in_use());
}

fn make_stage(migrated: bool) -> ironrdp::session::ActiveStage {
    use ironrdp_dvc::pdu::SoftSyncTunnelType;
    use ironrdp_svc::SvcProcessor as _;

    let mut drdynvc = ironrdp_dvc::DrdynvcClient::new();
    drdynvc.attach_established_dynamic_channel(7, TestChannel).unwrap();
    drdynvc.enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);
    if migrated {
        drdynvc.process(&soft_sync_request()).unwrap();
    }
    let mut static_channels = ironrdp_svc::StaticChannelSet::new();
    static_channels.insert(drdynvc);
    ironrdp::session::ActiveStageBuilder {
        static_channels,
        user_channel_id: 1001,
        io_channel_id: 1003,
        message_channel_id: Some(1005),
        share_id: 1,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build()
}

fn soft_sync_request() -> Vec<u8> {
    use ironrdp_dvc::pdu::{DrdynvcServerPdu, SoftSyncChannelList, SoftSyncRequestPdu, SoftSyncTunnelType};
    encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
        SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![7]),
    ])))
    .unwrap()
}

struct TestChannel;
ironrdp_core::impl_as_any!(TestChannel);

impl ironrdp_dvc::DvcProcessor for TestChannel {
    fn channel_name(&self) -> &str {
        "test-channel"
    }
    fn start(&mut self, _channel_id: u32) -> ironrdp_pdu::PduResult<Vec<ironrdp_dvc::DvcMessage>> {
        Ok(Vec::new())
    }
    fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<ironrdp_dvc::DvcMessage>> {
        Ok(Vec::new())
    }
}
impl ironrdp_dvc::DvcClientProcessor for TestChannel {}
