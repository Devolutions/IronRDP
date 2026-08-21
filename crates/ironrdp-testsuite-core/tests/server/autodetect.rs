use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};
use ironrdp_server::autodetect::{AutoDetectManager, AutoDetectOutcome};

/// Upper bound on ticks to drive while waiting for a bandwidth transaction:
/// generous enough to cover the pacing plus the window, tight enough that a
/// regression that stops `build_bandwidth_measure` from ever completing a
/// transaction fails the test instead of hanging CI.
const MAX_BANDWIDTH_TICKS: usize = 64;

/// Drives `build_bandwidth_measure` until it has produced both a Start and a
/// Stop, asserting the transaction shape along the way: exactly those two
/// PDUs, sharing one sequence number, with nothing in between. Pinning the
/// PDU types here is what would have caught a synthetic payload PDU sent
/// between them, which is off-spec for continuous detection per
/// [MS-RDPBCGR] 1.3.9. Returns the transaction's sequence number.
fn drive_bandwidth_start_and_stop(mgr: &mut AutoDetectManager) -> u16 {
    let mut start_sequence = None;
    for _ in 0..MAX_BANDWIDTH_TICKS {
        let Some(pdu) = mgr.build_bandwidth_measure() else {
            continue;
        };
        match (start_sequence, pdu) {
            (None, AutoDetectRequest::BandwidthMeasureStart { sequence_number, .. }) => {
                start_sequence = Some(sequence_number);
            }
            (Some(start), AutoDetectRequest::BandwidthMeasureStop { sequence_number, .. }) => {
                assert_eq!(
                    sequence_number, start,
                    "Start and Stop must share the transaction sequence"
                );
                return start;
            }
            (None, other) => panic!("expected Start first, got {other:?}"),
            (Some(_), other) => panic!("expected Stop after Start, got {other:?}"),
        }
    }
    panic!("bandwidth transaction did not complete within {MAX_BANDWIDTH_TICKS} ticks");
}

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
    assert_eq!(mgr.handle_response(&response, 20), AutoDetectOutcome::Rtt(20));
    assert_eq!(mgr.pending_count(), 0);
}

#[test]
fn unknown_sequence_returns_none() {
    let mut mgr = AutoDetectManager::new();
    let _ = mgr.send_rtt_request(0);

    let response = AutoDetectResponse::RttResponse { sequence_number: 999 };
    assert_eq!(mgr.handle_response(&response, 20), AutoDetectOutcome::Unmatched);
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

    let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert_eq!(
        mgr.handle_response(&results, 20),
        AutoDetectOutcome::Bandwidth(Some(80_000))
    );

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
fn bandwidth_kbps_reflects_the_last_completed_measurement() {
    let mut mgr = AutoDetectManager::new();
    assert!(mgr.bandwidth_kbps().is_none(), "nothing measured yet");

    let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert_eq!(
        mgr.handle_response(&results, 20),
        AutoDetectOutcome::Bandwidth(Some(80_000))
    );

    assert_eq!(mgr.bandwidth_kbps(), Some(80_000), "byte_count * 8 / time_delta_ms");
}

/// A second call to `build_bandwidth_measure` after Stop has been sent, while
/// the client's Bandwidth Measure Results is still outstanding, must not
/// start a new transaction. `drive_bandwidth_start_and_stop` already pins
/// that nothing is sent between Start and Stop themselves; this covers the
/// phase after Stop that it does not drive into.
#[test]
fn a_second_measurement_does_not_start_while_awaiting_results() {
    let mut mgr = AutoDetectManager::new();
    let _ = drive_bandwidth_start_and_stop(&mut mgr);

    for _ in 0..MAX_BANDWIDTH_TICKS {
        assert!(
            mgr.build_bandwidth_measure().is_none(),
            "no new transaction while the prior one awaits results"
        );
    }
}

/// A Bandwidth Measure Results whose sequence number does not match the
/// outstanding transaction must be ignored: no bandwidth recorded, and the
/// transaction stays outstanding rather than being cleared by an unrelated
/// reply.
#[test]
fn mismatched_bandwidth_sequence_is_ignored() {
    let mut mgr = AutoDetectManager::new();
    let req = mgr.send_rtt_request(0);
    let _ = mgr.handle_response(
        &AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        },
        20,
    );
    let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);

    // A reply for a different sequence must not be treated as completing the
    // outstanding transaction.
    let mismatched = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq.wrapping_add(1),
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert_eq!(mgr.handle_response(&mismatched, 20), AutoDetectOutcome::Unmatched);
    assert!(
        mgr.build_netchar_result(1_000).is_none(),
        "a mismatched reply must not have recorded a bandwidth figure"
    );

    // The real transaction is still outstanding, so its own reply still
    // completes it. If the mismatched reply above had been accepted, this
    // second reply would find no outstanding transaction to match and the
    // measurement would never complete.
    let matching = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert_eq!(
        mgr.handle_response(&matching, 20),
        AutoDetectOutcome::Bandwidth(Some(80_000))
    );
    assert!(
        mgr.build_netchar_result(1_000).is_some(),
        "the outstanding transaction's own reply must still complete it"
    );
}

/// A Bandwidth Measure Results with `time_delta_ms: 0` cannot compute a
/// figure. It must not be reported, and it must not leave a stale
/// `bandwidth_kbps` from an earlier successful measurement on the wire as if
/// it were current.
#[test]
fn zero_time_delta_ages_out_a_previous_bandwidth_figure() {
    let mut mgr = AutoDetectManager::new();
    let req = mgr.send_rtt_request(0);
    let _ = mgr.handle_response(
        &AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        },
        20,
    );

    // First measurement succeeds.
    let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);
    let good_results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert_eq!(
        mgr.handle_response(&good_results, 20),
        AutoDetectOutcome::Bandwidth(Some(80_000))
    );
    assert!(
        mgr.build_netchar_result(1_000).is_some(),
        "a real bandwidth figure is known"
    );

    // Second measurement fails: timeDelta 0.
    let req = mgr.send_rtt_request(1_000);
    let _ = mgr.handle_response(
        &AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        },
        1_020,
    );
    let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);
    let zero_delta_results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 0,
        byte_count: 100_000,
    };
    // Matched (it completes the outstanding transaction), but unusable: distinct from
    // an unmatched reply, and distinct from `Bandwidth(Some(_))`. A caller collapsing
    // this into "no measurement happened" would miss that the manager just cleared its
    // own figure right here.
    assert_eq!(
        mgr.handle_response(&zero_delta_results, 1_020),
        AutoDetectOutcome::Bandwidth(None)
    );

    // The RTT sample is fresh, but the old bandwidth figure must not ride
    // along as though it were still current.
    assert!(
        mgr.build_netchar_result(2_000).is_none(),
        "the aged-out bandwidth figure must withhold the result, not report the stale one"
    );
}

/// Two consecutive measurements landing on the identical kbps figure (plausible on a
/// stable link) must both be reported as a match. A caller that derives "did this
/// complete a measurement" from comparing the value before and after the call, rather
/// than from `handle_response`'s own return value, would see no change on the second
/// one and misclassify a real match as unmatched.
#[test]
fn identical_consecutive_bandwidth_measurements_are_both_reported_as_matched() {
    let mut mgr = AutoDetectManager::new();
    let req = mgr.send_rtt_request(0);
    let _ = mgr.handle_response(
        &AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        },
        20,
    );

    for now_ms in [20u64, 1_020] {
        let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);
        let results = AutoDetectResponse::BandwidthMeasureResults {
            sequence_number: bw_seq,
            response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
            time_delta_ms: 10,
            byte_count: 100_000,
        };
        assert_eq!(
            mgr.handle_response(&results, now_ms),
            AutoDetectOutcome::Bandwidth(Some(80_000)),
            "the same kbps figure twice in a row is still a match, not a repeat of nothing"
        );
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
fn autodetect_bandwidth_handle_defaults_to_sentinel() {
    use core::net::{Ipv4Addr, SocketAddr};
    use core::sync::atomic::Ordering;

    use ironrdp_server::RdpServer;

    let server = RdpServer::builder()
        .with_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build();

    assert_eq!(server.autodetect_bandwidth_handle().load(Ordering::Relaxed), u32::MAX);
}

#[test]
fn with_autodetect_bandwidth_handle_round_trips_the_same_arc() {
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
        .with_autodetect_bandwidth_handle(Arc::clone(&handle))
        .build();

    assert!(Arc::ptr_eq(&handle, &server.autodetect_bandwidth_handle()));
    // The server resets an injected handle to the sentinel at construction.
    assert_eq!(server.autodetect_bandwidth_handle().load(Ordering::Relaxed), u32::MAX);
    // The Arc is shared: mutating the original is visible through the server's handle.
    handle.store(42, Ordering::Relaxed);
    assert_eq!(server.autodetect_bandwidth_handle().load(Ordering::Relaxed), 42);
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

    let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert_eq!(
        mgr.handle_response(&results, 20),
        AutoDetectOutcome::Bandwidth(Some(80_000))
    );

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
    let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert_eq!(
        mgr.handle_response(&results, 20),
        AutoDetectOutcome::Bandwidth(Some(80_000))
    );

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

    let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert_eq!(
        mgr.handle_response(&results, 20),
        AutoDetectOutcome::Bandwidth(Some(80_000))
    );

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

    let bw_seq = drive_bandwidth_start_and_stop(&mut mgr);
    let results = AutoDetectResponse::BandwidthMeasureResults {
        sequence_number: bw_seq,
        response_type: ironrdp_pdu::rdp::autodetect::BW_RESULTS_CONTINUOUS,
        time_delta_ms: 10,
        byte_count: 100_000,
    };
    assert_eq!(
        mgr.handle_response(&results, 20),
        AutoDetectOutcome::Bandwidth(Some(80_000))
    );

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
