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
        mgr.build_netchar_result(0).is_none(),
        "no result should be produced before the network has been characterised"
    );
}

#[test]
fn netchar_result_withheld_until_bandwidth_is_known() {
    let mut mgr = AutoDetectManager::new();

    for _ in 0..3 {
        let req = mgr.send_rtt_request(0);
        let response = AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        };
        let _ = mgr.handle_response(&response, 20);
    }

    assert!(mgr.snapshot().is_some(), "RTT samples were recorded");
    assert!(
        mgr.build_netchar_result(20).is_none(),
        "RTT without a bandwidth measurement is not a characterisation of the network"
    );
}

#[test]
fn bandwidth_measure_transacts_and_enables_netchar() {
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

    match mgr
        .build_netchar_result(20)
        .expect("result once RTT and bandwidth are known")
    {
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

/// Mirrors the crate-private `NETCHAR_RESULT_MIN_INTERVAL_MS`, which is not
/// reachable across the crate boundary.
const NETCHAR_RESULT_INTERVAL_MS: u64 = 1_000;

/// Mirrors the crate-private `RTT_WINDOW_SIZE`.
const RTT_WINDOW: usize = 8;

/// The result is paced on its own clock, not on the probe cadence: a caller
/// probing far faster than the interval still emits at most one per interval.
#[test]
fn netchar_result_is_paced_independently_of_the_probe_cadence() {
    let mut mgr = AutoDetectManager::new();

    let req = mgr.send_rtt_request(0);
    let response = AutoDetectResponse::RttResponse {
        sequence_number: req.sequence_number(),
    };
    let _ = mgr.handle_response(&response, 20);

    let bw_seq = loop {
        if let Some(pdus) = mgr.build_bandwidth_measure() {
            break pdus[0].sequence_number();
        }
    };
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert!(mgr.handle_response(&results, 20).is_none());

    assert!(mgr.build_netchar_result(1_000).is_some(), "the first one is due");

    // Feed a fresh sample so only the interval is under test here.
    let req = mgr.send_rtt_request(1_000);
    let _ = mgr.handle_response(
        &AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        },
        1_020,
    );

    assert!(
        mgr.build_netchar_result(1_000 + NETCHAR_RESULT_INTERVAL_MS - 1)
            .is_none(),
        "a probe inside the interval does not emit another"
    );
    assert!(
        mgr.build_netchar_result(1_000 + NETCHAR_RESULT_INTERVAL_MS).is_some(),
        "the interval having elapsed, the next one is due"
    );
}

/// A client that stops answering must stop producing results. Samples never
/// age out of the window, so without this the last values would be
/// advertised forever.
#[test]
fn netchar_result_stops_when_the_client_stops_answering() {
    let mut mgr = AutoDetectManager::new();

    let req = mgr.send_rtt_request(0);
    let _ = mgr.handle_response(
        &AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        },
        20,
    );
    let bw_seq = loop {
        if let Some(pdus) = mgr.build_bandwidth_measure() {
            break pdus[0].sequence_number();
        }
    };
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert!(mgr.handle_response(&results, 20).is_none());

    let mut now = 1_000;
    assert!(mgr.build_netchar_result(now).is_some(), "the first one is due");

    // The client has gone quiet. The interval keeps elapsing and the window
    // still holds its samples, but there is nothing new to report.
    for _ in 0..5 {
        now += NETCHAR_RESULT_INTERVAL_MS;
        assert!(
            mgr.build_netchar_result(now).is_none(),
            "no response has arrived since the last result"
        );
    }
    assert!(mgr.snapshot().is_some(), "the samples are still there for the embedder");

    // One reply, and reporting resumes.
    let req = mgr.send_rtt_request(now);
    let _ = mgr.handle_response(
        &AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        },
        now + 20,
    );
    now += NETCHAR_RESULT_INTERVAL_MS;
    assert!(
        mgr.build_netchar_result(now).is_some(),
        "a fresh sample resumes reporting"
    );
}

/// [MS-RDPBCGR] 2.2.14.1.5 defines baseRTT as the lowest detected RTT, with no
/// window scoping, so it must not rise when a low sample ages out of the
/// window. This is the reviewer's sequence: one 5 ms sample, then eight of
/// 100 ms, which evicts it.
#[test]
fn base_rtt_is_the_session_low_not_the_window_low() {
    let mut mgr = AutoDetectManager::new();

    let sample = |mgr: &mut AutoDetectManager, rtt: u64| {
        let req = mgr.send_rtt_request(0);
        let _ = mgr.handle_response(
            &AutoDetectResponse::RttResponse {
                sequence_number: req.sequence_number(),
            },
            rtt,
        );
    };

    sample(&mut mgr, 5);
    for _ in 0..RTT_WINDOW {
        sample(&mut mgr, 100);
    }

    assert_eq!(
        mgr.snapshot().expect("samples recorded").min_ms,
        100,
        "the 5 ms sample has aged out of the window"
    );

    let bw_seq = loop {
        if let Some(pdus) = mgr.build_bandwidth_measure() {
            break pdus[0].sequence_number();
        }
    };
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert!(mgr.handle_response(&results, 20).is_none());

    match mgr.build_netchar_result(1_000).expect("RTT and bandwidth are known") {
        AutoDetectRequest::NetworkCharacteristicsResult { base_rtt_ms, .. } => {
            assert_eq!(base_rtt_ms, Some(5), "baseRTT stays at the lowest ever detected");
        }
        other => panic!("expected NetworkCharacteristicsResult, got {other:?}"),
    }
}

/// Pins which measured quantity lands in which wire field.
///
/// The session minimum, the window minimum and the window average are all
/// different here, so no swap between them leaves both assertions holding.
#[test]
fn netchar_result_maps_each_measurement_to_its_own_field() {
    let mut mgr = AutoDetectManager::new();

    let sample = |mgr: &mut AutoDetectManager, rtt: u64| {
        let req = mgr.send_rtt_request(0);
        let _ = mgr.handle_response(
            &AutoDetectResponse::RttResponse {
                sequence_number: req.sequence_number(),
            },
            rtt,
        );
    };

    sample(&mut mgr, 5);
    for rtt in [10, 20, 30, 40, 50, 60, 70, 80] {
        sample(&mut mgr, rtt);
    }

    let snap = mgr.snapshot().expect("samples recorded");
    assert_eq!(snap.min_ms, 10, "the 5 ms sample has left the window");
    assert_eq!(snap.avg_ms, 45, "average of 10 through 80");

    let bw_seq = loop {
        if let Some(pdus) = mgr.build_bandwidth_measure() {
            break pdus[0].sequence_number();
        }
    };
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert!(mgr.handle_response(&results, 20).is_none());

    match mgr.build_netchar_result(1_000).expect("RTT and bandwidth are known") {
        AutoDetectRequest::NetworkCharacteristicsResult {
            base_rtt_ms,
            bandwidth_kbps,
            average_rtt_ms,
            ..
        } => {
            assert_eq!(base_rtt_ms, Some(5), "baseRTT is the session low, not the window low");
            assert_eq!(average_rtt_ms, 45, "averageRTT is the window average");
            assert_eq!(bandwidth_kbps, Some(80_000), "byte_count * 8 / time_delta_ms");
        }
        other => panic!("expected NetworkCharacteristicsResult, got {other:?}"),
    }
}
