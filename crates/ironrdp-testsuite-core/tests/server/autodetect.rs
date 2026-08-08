use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};
use ironrdp_server::autodetect::AutoDetectManager;

#[test]
fn rtt_request_increments_sequence() {
    let mut mgr = AutoDetectManager::new();
    let req1 = mgr.send_rtt_request(0);
    let req2 = mgr.send_rtt_request(0);
    assert_eq!(req1.sequence_number(), 0);
    assert_eq!(req2.sequence_number(), 1);
    assert_eq!(mgr.pending_count(), 2);
}

#[test]
fn rtt_response_returns_latency() {
    let mut mgr = AutoDetectManager::new();
    let req = mgr.send_rtt_request(0);

    let response = AutoDetectResponse::RttResponse {
        sequence_number: req.sequence_number(),
    };
    // Both timestamps come from the caller, so the measurement is exact rather than
    // dependent on how fast the test happens to run.
    assert_eq!(mgr.handle_response(&response, 20), Some(20));
    assert_eq!(mgr.pending_count(), 0);
}

#[test]
fn unknown_sequence_returns_none() {
    let mut mgr = AutoDetectManager::new();
    let _ = mgr.send_rtt_request(0);

    let response = AutoDetectResponse::RttResponse { sequence_number: 999 };
    assert!(mgr.handle_response(&response, 20).is_none());
    assert_eq!(mgr.pending_count(), 1, "original probe should remain");
}

#[test]
fn snapshot_none_without_measurements() {
    let mgr = AutoDetectManager::new();
    assert!(mgr.snapshot().is_none());
}

#[test]
fn snapshot_reflects_measurements() {
    let mut mgr = AutoDetectManager::new();

    for (sent_at, rtt) in [(0u64, 10u64), (100, 20), (200, 30)] {
        let req = mgr.send_rtt_request(sent_at);
        let response = AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        };
        let _ = mgr.handle_response(&response, sent_at + rtt);
    }

    let snap = mgr.snapshot().expect("should have data");
    assert_eq!(snap.sample_count, 3);
    assert_eq!(snap.min_ms, 10);
    assert_eq!(snap.max_ms, 30);
    assert_eq!(snap.avg_ms, 20);
}

#[test]
fn netchar_result_none_without_measurements() {
    let mut mgr = AutoDetectManager::new();
    assert!(
        mgr.build_netchar_result().is_none(),
        "no result should be produced before any RTT sample"
    );
}

#[test]
fn netchar_result_reports_measured_rtt() {
    let mut mgr = AutoDetectManager::new();

    for _ in 0..3 {
        let req = mgr.send_rtt_request(0);
        let response = AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        };
        let _ = mgr.handle_response(&response, 20);
    }

    let snap = mgr.snapshot().expect("should have data");
    match mgr.build_netchar_result().expect("result once samples exist") {
        AutoDetectRequest::NetworkCharacteristicsResult {
            base_rtt_ms,
            bandwidth_kbps,
            average_rtt_ms,
            ..
        } => {
            assert_eq!(base_rtt_ms, Some(snap.min_ms), "baseRTT is the lowest observed RTT");
            assert_eq!(average_rtt_ms, snap.avg_ms, "averageRTT matches the window average");
            assert_eq!(bandwidth_kbps, None, "RTT-only variant omits bandwidth");
        }
        other => panic!("expected NetworkCharacteristicsResult, got {other:?}"),
    }
}

#[test]
fn bandwidth_measure_transacts_and_upgrades_netchar() {
    let mut mgr = AutoDetectManager::new();
    let req = mgr.send_rtt_request(0);
    let _ = mgr.handle_response(
        &AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        },
        20,
    );

    // Drive a bandwidth measurement to completion (paced internally).
    let pdus = loop {
        if let Some(p) = mgr.build_bandwidth_measure() {
            break p;
        }
    };
    assert_eq!(
        pdus[0].sequence_number(),
        pdus[2].sequence_number(),
        "Start and Stop share the transaction sequence"
    );
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: pdus[0].sequence_number(),
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert!(mgr.handle_response(&results, 20).is_none());

    match mgr.build_netchar_result().expect("result once samples exist") {
        AutoDetectRequest::NetworkCharacteristicsResult { bandwidth_kbps, .. } => {
            assert_eq!(bandwidth_kbps, Some(80_000), "byte_count * 8 / time_delta_ms");
        }
        other => panic!("expected NetworkCharacteristicsResult, got {other:?}"),
    }
}

#[test]
fn sequence_number_wraps_at_u16_max() {
    let mut mgr = AutoDetectManager::new();
    // Advance sequence counter through all values, resolving each probe immediately
    // to avoid growing pending_probes to 65k entries.
    for _ in 0..u16::MAX {
        let req = mgr.send_rtt_request(0);
        let response = AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        };
        let _ = mgr.handle_response(&response, 20);
    }
    let req = mgr.send_rtt_request(0);
    assert_eq!(req.sequence_number(), u16::MAX);

    let req2 = mgr.send_rtt_request(0);
    assert_eq!(req2.sequence_number(), 0, "should wrap around");
}

#[test]
fn autodetect_rtt_handle_defaults_to_sentinel() {
    use core::net::{Ipv4Addr, SocketAddr};
    use core::sync::atomic::Ordering;

    use ironrdp_server::RdpServer;

    let server = RdpServer::builder()
        .with_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build();

    assert_eq!(server.autodetect_rtt_handle().load(Ordering::Relaxed), u32::MAX);
}

#[test]
fn with_autodetect_rtt_handle_round_trips_the_same_arc() {
    use core::net::{Ipv4Addr, SocketAddr};
    use core::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use ironrdp_server::RdpServer;

    let handle = Arc::new(AtomicU32::new(42));
    let server = RdpServer::builder()
        .with_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .with_autodetect_rtt_handle(Arc::clone(&handle))
        .build();

    assert!(Arc::ptr_eq(&handle, &server.autodetect_rtt_handle()));
    // The server resets an injected handle to the sentinel at construction.
    assert_eq!(server.autodetect_rtt_handle().load(Ordering::Relaxed), u32::MAX);
    // The Arc is shared: mutating the original is visible through the server's handle.
    handle.store(42, Ordering::Relaxed);
    assert_eq!(server.autodetect_rtt_handle().load(Ordering::Relaxed), 42);
}

#[test]
fn stale_probe_expiry() {
    let mut mgr = AutoDetectManager::new();
    let _ = mgr.send_rtt_request(0);
    assert_eq!(mgr.pending_count(), 1);

    mgr.expire_stale_probes(1, 0);
    assert_eq!(mgr.pending_count(), 0);
}
