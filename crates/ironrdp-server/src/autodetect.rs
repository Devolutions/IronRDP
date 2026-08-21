//! Server-side auto-detect (RTT and bandwidth measurement) per [MS-RDPBCGR 2.2.14].
//!
//! The server periodically sends RTT Measure Request PDUs and records the
//! round-trip time from the client's response, and periodically brackets
//! ordinary traffic with a Bandwidth Measure Start/Stop pair so the client
//! can report the bandwidth it observed. RTT results are exposed via
//! [`AutoDetectManager::snapshot()`]; both RTT and bandwidth are reported to
//! the client via [`AutoDetectManager::build_netchar_result()`].
//!
//! [MS-RDPBCGR 2.2.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dc672839-4f4e-40b1-a71c-cd6a959baa38

use std::collections::VecDeque;

use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};

/// Number of RTT samples to retain for averaging.
const RTT_WINDOW_SIZE: usize = 8;

/// Probes older than this are discarded as unresponsive.
pub(crate) const RTT_PROBE_MAX_AGE_MS: u64 = 30_000;

/// Run a bandwidth measurement once every this many RTT ticks. Bandwidth
/// changes far more slowly than RTT, so it is sampled less often than the
/// per-tick RTT probe.
const BW_MEASURE_INTERVAL_TICKS: u32 = 8;

/// Number of RTT ticks a bandwidth window stays open between Start and Stop.
///
/// [MS-RDPBCGR] 1.3.9 and 2.2.14.2.2 scope the Bandwidth Measure Payload
/// message to the connect-time transaction only; after the RDP Connection
/// Sequence, "the PDUs sent from server to client (between start and stop
/// messages) replace the payload messages". So the window has to stay open
/// long enough for ordinary traffic to fill it, rather than closing
/// immediately the way a synthetic payload would allow.
const BW_WINDOW_TICKS: u32 = 4;

/// Minimum spacing between Network Characteristics Result PDUs.
///
/// The reported figures are averaged over a window and change far more slowly
/// than the RTT probe cadence, so pacing the result independently keeps an
/// aggressive probe rate from turning into an equally aggressive stream of
/// unsolicited PDUs.
const NETCHAR_RESULT_MIN_INTERVAL_MS: u64 = 1_000;

/// Server-side auto-detect state machine.
///
/// Tracks outstanding RTT probes and computes round-trip statistics from
/// client responses. Call [`send_rtt_request()`](Self::send_rtt_request) to
/// generate a probe, then [`handle_response()`](Self::handle_response) when
/// the client replies.
///
/// The manager reads no clock of its own. Every method that needs the current time
/// takes `now_ms`, a caller-supplied monotonic millisecond counter whose epoch is
/// arbitrary as long as it is consistent across calls. Keeping time out of the state
/// machine makes RTT measurement exactly testable, leaves the type usable from targets
/// where the standard library has no clock (`std::time::Instant::now` panics on
/// `wasm32-unknown-unknown`), and keeps the crate a candidate for the no-I/O rules the
/// Core Tier crates follow.
pub struct AutoDetectManager {
    next_sequence: u16,
    /// Outstanding probes as `(sequence_number, sent_at_ms)`.
    pending_probes: Vec<(u16, u64)>,
    rtt_samples: VecDeque<u32>,
    /// State of an in-flight bandwidth measurement, if any. A new measurement
    /// is not started while one is outstanding.
    pending_bw: Option<PendingBandwidth>,
    /// Most recently measured bandwidth in kilobits per second.
    bandwidth_kbps: Option<u32>,
    /// Counts RTT ticks to pace bandwidth measurements (see [`BW_MEASURE_INTERVAL_TICKS`]).
    bw_tick_count: u32,
    /// When the last Network Characteristics Result was built, for pacing.
    last_netchar_result_ms: Option<u64>,
    /// Lowest RTT observed over the whole session, for the wire `baseRTT`.
    ///
    /// [MS-RDPBCGR] 2.2.14.1.5 defines that field as "the lowest detected
    /// round-trip time" with no window scoping, so it is tracked separately from
    /// the sliding window behind [`snapshot()`](Self::snapshot). A floor derived
    /// from the window can rise once a low sample is evicted, which would make
    /// the baseRTT / averageRTT pair unusable for the queueing-delay difference
    /// a client computes from it.
    min_rtt_ms: Option<u32>,
    /// Set when an RTT response updates the measured figures, cleared when a
    /// Network Characteristics Result reports them.
    ///
    /// Without it a client that stops answering leaves the last window values
    /// being advertised indefinitely, since they never age out of `rtt_samples`.
    rtt_is_fresh: bool,
    /// Set when a bandwidth measurement completes, cleared when a Network
    /// Characteristics Result reports it.
    ///
    /// Tracked separately from [`rtt_is_fresh`](Self::rtt_is_fresh): bandwidth
    /// measurements complete far less often than RTT samples arrive, so an RTT
    /// response alone must not re-arm reporting of a bandwidth figure that may
    /// be several measurements stale.
    bandwidth_is_fresh: bool,
}

/// Result of [`AutoDetectManager::handle_response()`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoDetectOutcome {
    /// An RTT Measure Response matched a pending probe; carries the measured RTT
    /// in milliseconds.
    Rtt(u32),
    /// A Bandwidth Measure Results matched the pending transaction. `None` means
    /// the measurement completed without a usable figure, and any previously
    /// stored bandwidth was cleared rather than left in place.
    Bandwidth(Option<u32>),
    /// The response did not match any pending probe or transaction.
    Unmatched,
}

/// State of an in-flight continuous bandwidth measurement.
enum PendingBandwidth {
    /// Start has been sent; the window stays open for this many more ticks
    /// before Stop is sent, so ordinary traffic can fill it.
    Open { sequence: u16, ticks_remaining: u32 },
    /// Stop has been sent; awaiting the client's Bandwidth Measure Results.
    AwaitingResults { sequence: u16 },
}

impl AutoDetectManager {
    pub fn new() -> Self {
        Self {
            next_sequence: 0,
            pending_probes: Vec::new(),
            rtt_samples: VecDeque::with_capacity(RTT_WINDOW_SIZE),
            pending_bw: None,
            bandwidth_kbps: None,
            bw_tick_count: 0,
            last_netchar_result_ms: None,
            rtt_is_fresh: false,
            bandwidth_is_fresh: false,
            min_rtt_ms: None,
        }
    }

    /// Generate an RTT Measure Request PDU for continuous detection.
    ///
    /// The caller must encode and send the returned [`AutoDetectRequest`] on
    /// the MCS message channel, framed by a `SEC_AUTODETECT_REQ` security
    /// header ([MS-RDPBCGR] 2.2.14.3). `now_ms` is recorded as the send time
    /// and is what [`handle_response()`](Self::handle_response) measures against.
    pub fn send_rtt_request(&mut self, now_ms: u64) -> AutoDetectRequest {
        let seq = self.next_sequence;
        self.next_sequence = seq.wrapping_add(1);
        self.pending_probes.push((seq, now_ms));
        AutoDetectRequest::rtt_continuous(seq)
    }

    /// Advance the bandwidth measurement state machine by one tick, returning
    /// a PDU to send when the machine has one, or `None` otherwise.
    ///
    /// A measurement is a Start sent when one is due (paced to once every
    /// [`BW_MEASURE_INTERVAL_TICKS`] calls and suppressed while a prior one is
    /// outstanding), followed by a Stop sent [`BW_WINDOW_TICKS`] calls later.
    /// No payload PDU is sent between them: per [MS-RDPBCGR] 1.3.9, Continuous
    /// Auto-Detection's server-to-client message set for bandwidth measurement
    /// is Start and Stop only, and 2.2.14.2.2 states that after the RDP
    /// Connection Sequence the ordinary PDUs the server sends between them
    /// are what the client counts. The caller sends whichever PDU this
    /// returns on the MCS message channel; the client eventually replies with
    /// a Bandwidth Measure Results PDU, processed by
    /// [`handle_response()`](Self::handle_response).
    pub fn build_bandwidth_measure(&mut self) -> Option<AutoDetectRequest> {
        match &mut self.pending_bw {
            Some(PendingBandwidth::Open {
                sequence,
                ticks_remaining,
            }) => {
                if *ticks_remaining == 0 {
                    let sequence = *sequence;
                    self.pending_bw = Some(PendingBandwidth::AwaitingResults { sequence });
                    return Some(AutoDetectRequest::bw_stop_continuous(sequence));
                }
                *ticks_remaining -= 1;
                None
            }
            Some(PendingBandwidth::AwaitingResults { .. }) => None,
            None => {
                self.bw_tick_count = self.bw_tick_count.wrapping_add(1);
                if !self.bw_tick_count.is_multiple_of(BW_MEASURE_INTERVAL_TICKS) {
                    return None;
                }
                let seq = self.next_sequence;
                self.next_sequence = seq.wrapping_add(1);
                self.pending_bw = Some(PendingBandwidth::Open {
                    sequence: seq,
                    ticks_remaining: BW_WINDOW_TICKS,
                });
                Some(AutoDetectRequest::bw_start_continuous(seq))
            }
        }
    }

    /// Build a Network Characteristics Result reporting the measured network.
    ///
    /// Returns `None` until the network has actually been characterised, which
    /// means both an RTT sample and a completed bandwidth measurement. A result
    /// carrying RTT alone reports part of the picture as though it were the
    /// whole, so it is not sent; the same is true of the reference
    /// implementations, which return early with no bandwidth figure to report.
    ///
    /// Also returns `None` when one was sent less than
    /// [`NETCHAR_RESULT_MIN_INTERVAL_MS`] ago. The RTT probe cadence is set by
    /// the caller and can be far faster than the rate at which the reported
    /// figures meaningfully change, so the result is paced independently rather
    /// than emitted once per probe.
    ///
    /// Finally, returns `None` unless a client response has updated either
    /// figure since the last result. RTT samples do not age out of the
    /// window, and a completed bandwidth measurement is never invalidated by
    /// a later failed one on its own, so a client that stops answering would
    /// otherwise leave the last values being advertised forever. `snapshot()`
    /// still reports them, where staleness is the embedder's to judge, but
    /// they are not put on the wire as a current claim; a bandwidth
    /// measurement that completes without a usable figure clears
    /// `bandwidth_kbps` rather than leaving the previous one in place, so a
    /// run of failures withholds the result entirely instead of reporting an
    /// increasingly stale number.
    ///
    /// `baseRTT` is the lowest RTT seen over the whole session, not the lowest in
    /// the current window, so it never rises. `averageRTT` is the window average
    /// and does track current conditions; the difference between the two is what
    /// a client reads as queueing delay.
    ///
    /// Like [`send_rtt_request()`](Self::send_rtt_request), the caller sends the
    /// returned PDU on the MCS message channel. The client does not reply to it.
    pub fn build_netchar_result(&mut self, now_ms: u64) -> Option<AutoDetectRequest> {
        if !self.rtt_is_fresh && !self.bandwidth_is_fresh {
            return None;
        }
        let bandwidth_kbps = self.bandwidth_kbps?;
        let base_rtt_ms = self.min_rtt_ms?;
        let snapshot = self.snapshot()?;

        if let Some(last) = self.last_netchar_result_ms {
            if now_ms.saturating_sub(last) < NETCHAR_RESULT_MIN_INTERVAL_MS {
                return None;
            }
        }
        self.last_netchar_result_ms = Some(now_ms);
        self.rtt_is_fresh = false;
        self.bandwidth_is_fresh = false;

        let seq = self.next_sequence;
        self.next_sequence = seq.wrapping_add(1);
        Some(AutoDetectRequest::netchar_result(
            seq,
            base_rtt_ms,
            bandwidth_kbps,
            snapshot.avg_ms,
        ))
    }

    /// Process an Auto-Detect Response from the client.
    ///
    /// For an RTT Measure Response, records the sample and returns
    /// [`AutoDetectOutcome::Rtt`] with the measured RTT in milliseconds. For a
    /// Bandwidth Measure Results, records the computed bandwidth internally and
    /// returns [`AutoDetectOutcome::Bandwidth`]; its payload is `None` when the
    /// measurement completed without a usable figure, in which case any
    /// previously stored bandwidth was just cleared (see the note on
    /// [`build_netchar_result()`](Self::build_netchar_result)). Returns
    /// [`AutoDetectOutcome::Unmatched`] for an unexpected or unmatched response.
    ///
    /// `now_ms` is the receipt time on the same clock passed to
    /// [`send_rtt_request()`](Self::send_rtt_request).
    pub fn handle_response(&mut self, response: &AutoDetectResponse, now_ms: u64) -> AutoDetectOutcome {
        match response {
            AutoDetectResponse::RttResponse { sequence_number } => {
                let Some(idx) = self.pending_probes.iter().position(|(s, _)| *s == *sequence_number) else {
                    return AutoDetectOutcome::Unmatched;
                };
                let (_, sent_at_ms) = self.pending_probes.remove(idx);

                // Saturating rather than wrapping: a caller whose clock went backwards gets
                // a zero sample, not a nonsense one near u32::MAX.
                let rtt_ms = u32::try_from(now_ms.saturating_sub(sent_at_ms)).unwrap_or(u32::MAX);

                if self.rtt_samples.len() >= RTT_WINDOW_SIZE {
                    self.rtt_samples.pop_front();
                }
                self.rtt_samples.push_back(rtt_ms);
                self.min_rtt_ms = Some(self.min_rtt_ms.map_or(rtt_ms, |m| m.min(rtt_ms)));
                self.rtt_is_fresh = true;

                AutoDetectOutcome::Rtt(rtt_ms)
            }
            AutoDetectResponse::BandwidthMeasureResults { sequence_number, .. } => {
                let awaiting = matches!(
                    self.pending_bw,
                    Some(PendingBandwidth::AwaitingResults { sequence }) if sequence == *sequence_number
                );
                if !awaiting {
                    return AutoDetectOutcome::Unmatched;
                }
                self.pending_bw = None;
                // A transaction that completes without a usable figure clears the
                // previous one rather than leaving it in place, so a run of failures
                // withholds the result (see `build_netchar_result`) instead of
                // reporting an increasingly stale bandwidth.
                self.bandwidth_kbps = response.computed_bandwidth_kbps();
                if self.bandwidth_kbps.is_some() {
                    self.bandwidth_is_fresh = true;
                }
                AutoDetectOutcome::Bandwidth(self.bandwidth_kbps)
            }
            _ => AutoDetectOutcome::Unmatched,
        }
    }

    /// Get current RTT statistics, or `None` if no measurements yet.
    pub fn snapshot(&self) -> Option<RttSnapshot> {
        if self.rtt_samples.is_empty() {
            return None;
        }

        let min = *self.rtt_samples.iter().min().unwrap_or(&0);
        let max = *self.rtt_samples.iter().max().unwrap_or(&0);
        let sum: u64 = self.rtt_samples.iter().map(|&v| u64::from(v)).sum();
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "average of u32 samples fits in u32"
        )]
        let avg = (sum / self.rtt_samples.len() as u64) as u32;

        Some(RttSnapshot {
            min_ms: min,
            max_ms: max,
            avg_ms: avg,
            sample_count: self.rtt_samples.len(),
        })
    }

    /// Number of outstanding probes awaiting response.
    pub fn pending_count(&self) -> usize {
        self.pending_probes.len()
    }

    /// Most recently measured bandwidth in kilobits per second, or `None` if
    /// no bandwidth measurement has completed yet. Unlike
    /// [`Self::build_netchar_result`], this is not gated on freshness or
    /// pacing: it reports the last completed figure even if a subsequent
    /// failed measurement or an idle client has left it unrefreshed for a
    /// while, the same relationship [`Self::snapshot`] has to RTT. Staleness
    /// is the caller's to judge.
    pub fn bandwidth_kbps(&self) -> Option<u32> {
        self.bandwidth_kbps
    }

    /// Discard probes older than `max_age_ms` to prevent unbounded growth.
    ///
    /// `now_ms` is on the same clock passed to
    /// [`send_rtt_request()`](Self::send_rtt_request).
    pub fn expire_stale_probes(&mut self, now_ms: u64, max_age_ms: u64) {
        self.pending_probes
            .retain(|(_, sent_at_ms)| now_ms.saturating_sub(*sent_at_ms) < max_age_ms);
    }
}

impl Default for AutoDetectManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of RTT measurement results.
#[derive(Debug, Clone, Copy)]
pub struct RttSnapshot {
    /// Minimum RTT in milliseconds over the current window.
    ///
    /// Not the `baseRTT` sent on the wire, which is the session-lifetime lowest
    /// per [MS-RDPBCGR] 2.2.14.1.5. This one can rise as low samples age out.
    pub min_ms: u32,
    /// Maximum observed RTT in milliseconds.
    pub max_ms: u32,
    /// Average RTT in milliseconds over the sample window.
    pub avg_ms: u32,
    /// Number of samples in the current window.
    pub sample_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn rtt_response_computes_latency() {
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
    fn snapshot_returns_none_without_data() {
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

        let snap = mgr.snapshot().expect("should have data after 3 measurements");
        assert_eq!(snap.sample_count, 3);
        assert_eq!(snap.min_ms, 10);
        assert_eq!(snap.max_ms, 30);
        assert_eq!(snap.avg_ms, 20);
    }

    /// A caller whose clock ran backwards between the request and the response must
    /// produce a zero sample. Wrapping subtraction here would yield a value near
    /// `u32::MAX` and poison every statistic drawn from the window.
    #[test]
    fn backwards_clock_yields_a_zero_sample() {
        let mut mgr = AutoDetectManager::new();
        let req = mgr.send_rtt_request(1000);

        let response = AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        };
        assert_eq!(mgr.handle_response(&response, 400), AutoDetectOutcome::Rtt(0));

        // The zero has to reach the window, not just the return value, since the
        // snapshot is what the peer eventually sees.
        let snap = mgr.snapshot().expect("one measurement was recorded");
        assert_eq!(snap.min_ms, 0);
        assert_eq!(snap.max_ms, 0);
        assert_eq!(snap.avg_ms, 0);
    }

    /// The same backwards clock reaches expiry, where the saturation has the opposite
    /// shape: an age of zero is below any maximum, so the probe stays pending.
    /// Wrapping subtraction would make it look older than any limit and drop a probe
    /// whose response is still in flight.
    #[test]
    fn backwards_clock_keeps_the_probe_pending() {
        let mut mgr = AutoDetectManager::new();
        let _ = mgr.send_rtt_request(1000);

        mgr.expire_stale_probes(400, 100);
        assert_eq!(mgr.pending_count(), 1, "a probe from the future is not stale");
    }

    /// A gap wider than `u32::MAX` milliseconds (about 49.7 days) clamps rather than
    /// truncating to the low 32 bits, which would report a small RTT for an enormous one.
    #[test]
    fn an_enormous_gap_clamps_to_u32_max() {
        let mut mgr = AutoDetectManager::new();
        let req = mgr.send_rtt_request(0);

        let response = AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        };
        assert_eq!(
            mgr.handle_response(&response, u64::from(u32::MAX) + 1),
            AutoDetectOutcome::Rtt(u32::MAX)
        );
    }

    #[test]
    fn sequence_number_wraps() {
        let mut mgr = AutoDetectManager::new();
        mgr.next_sequence = u16::MAX;
        let req = mgr.send_rtt_request(0);
        assert_eq!(req.sequence_number(), u16::MAX);

        let req2 = mgr.send_rtt_request(0);
        assert_eq!(req2.sequence_number(), 0, "should wrap around");
    }

    #[test]
    fn stale_probe_expiry() {
        let mut mgr = AutoDetectManager::new();
        let _ = mgr.send_rtt_request(0);
        assert_eq!(mgr.pending_count(), 1);

        mgr.expire_stale_probes(1, 0);
        assert_eq!(mgr.pending_count(), 0);
    }
}
