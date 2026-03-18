use ironrdp_pdu::rdp::autodetect::AutoDetectResponse;
use ironrdp_server::autodetect::AutoDetectManager;

#[test]
fn rtt_request_increments_sequence() {
    let mut mgr = AutoDetectManager::new();
    let req1 = mgr.send_rtt_request();
    let req2 = mgr.send_rtt_request();
    assert_eq!(req1.sequence_number(), 0);
    assert_eq!(req2.sequence_number(), 1);
    assert_eq!(mgr.pending_count(), 2);
}

#[test]
fn rtt_response_returns_latency() {
    let mut mgr = AutoDetectManager::new();
    let req = mgr.send_rtt_request();

    let response = AutoDetectResponse::RttResponse {
        sequence_number: req.sequence_number(),
    };
    let rtt = mgr.handle_response(&response);
    assert!(rtt.is_some(), "should match the outstanding probe");
    assert_eq!(mgr.pending_count(), 0);
}

#[test]
fn unknown_sequence_returns_none() {
    let mut mgr = AutoDetectManager::new();
    let _ = mgr.send_rtt_request();

    let response = AutoDetectResponse::RttResponse { sequence_number: 999 };
    assert!(mgr.handle_response(&response).is_none());
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

    for _ in 0..3 {
        let req = mgr.send_rtt_request();
        let response = AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        };
        let _ = mgr.handle_response(&response);
    }

    let snap = mgr.snapshot().expect("should have data");
    assert_eq!(snap.sample_count, 3);
    // RTT should be ~0ms (same-process send/receive)
    assert!(snap.avg_ms < 100);
}

#[test]
fn sequence_number_wraps_at_u16_max() {
    let mut mgr = AutoDetectManager::new();
    // Advance sequence counter through all values, resolving each probe immediately
    // to avoid growing pending_probes to 65k entries.
    for _ in 0..u16::MAX {
        let req = mgr.send_rtt_request();
        let response = AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        };
        let _ = mgr.handle_response(&response);
    }
    let req = mgr.send_rtt_request();
    assert_eq!(req.sequence_number(), u16::MAX);

    let req2 = mgr.send_rtt_request();
    assert_eq!(req2.sequence_number(), 0, "should wrap around");
}

#[test]
fn stale_probe_expiry() {
    let mut mgr = AutoDetectManager::new();
    let _ = mgr.send_rtt_request();
    assert_eq!(mgr.pending_count(), 1);

    mgr.expire_stale_probes(core::time::Duration::ZERO);
    assert_eq!(mgr.pending_count(), 0);
}
