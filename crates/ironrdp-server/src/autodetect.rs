//! Server-side auto-detect (RTT measurement) per [MS-RDPBCGR 2.2.14].
//!
//! The server periodically sends RTT Measure Request PDUs and records the
//! round-trip time from the client's response. Results are exposed via
//! [`AutoDetectManager::snapshot()`].
//!
//! [MS-RDPBCGR 2.2.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dc672839-4f4e-40b1-a71c-cd6a959baa38

use std::collections::VecDeque;

use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, AutoDetectResponse};

/// Number of RTT samples to retain for averaging.
const RTT_WINDOW_SIZE: usize = 8;

/// Probes older than this are discarded as unresponsive.
pub(crate) const RTT_PROBE_MAX_AGE_MS: u64 = 30_000;

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
}

impl AutoDetectManager {
    pub fn new() -> Self {
        Self {
            next_sequence: 0,
            pending_probes: Vec::new(),
            rtt_samples: VecDeque::with_capacity(RTT_WINDOW_SIZE),
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

    /// Build a Network Characteristics Result reporting the measured RTT.
    ///
    /// Returns `None` until at least one RTT sample has been recorded. The
    /// result carries baseRTT (lowest observed) and averageRTT over the current
    /// window; bandwidth is omitted. Like [`send_rtt_request()`](Self::send_rtt_request),
    /// the caller sends the returned PDU on the MCS message channel. The client
    /// does not reply to it.
    pub fn build_netchar_result(&mut self) -> Option<AutoDetectRequest> {
        let snapshot = self.snapshot()?;
        let seq = self.next_sequence;
        self.next_sequence = seq.wrapping_add(1);
        Some(AutoDetectRequest::netchar_result_rtt(
            seq,
            snapshot.min_ms,
            snapshot.avg_ms,
        ))
    }

    /// Process an RTT Measure Response from the client.
    ///
    /// Returns the measured RTT in milliseconds if the sequence number
    /// matches an outstanding probe, or `None` if it was unexpected.
    /// `now_ms` is the receipt time on the same clock passed to
    /// [`send_rtt_request()`](Self::send_rtt_request).
    pub fn handle_response(&mut self, response: &AutoDetectResponse, now_ms: u64) -> Option<u32> {
        let AutoDetectResponse::RttResponse { sequence_number } = response else {
            return None;
        };

        let idx = self.pending_probes.iter().position(|(s, _)| *s == *sequence_number)?;
        let (_, sent_at_ms) = self.pending_probes.remove(idx);

        // Saturating rather than wrapping: a caller whose clock went backwards gets a
        // zero sample, not a nonsense one near u32::MAX.
        let rtt_ms = u32::try_from(now_ms.saturating_sub(sent_at_ms)).unwrap_or(u32::MAX);

        if self.rtt_samples.len() >= RTT_WINDOW_SIZE {
            self.rtt_samples.pop_front();
        }
        self.rtt_samples.push_back(rtt_ms);

        Some(rtt_ms)
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
    /// Minimum observed RTT in milliseconds.
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
        assert_eq!(mgr.handle_response(&response, 400), Some(0));

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
        assert_eq!(mgr.handle_response(&response, u64::from(u32::MAX) + 1), Some(u32::MAX));
    }

    #[test]
    fn netchar_result_none_without_samples() {
        let mut mgr = AutoDetectManager::new();
        assert!(mgr.build_netchar_result().is_none());
    }

    #[test]
    fn netchar_result_reports_measured_rtt() {
        let mut mgr = AutoDetectManager::new();

        let req = mgr.send_rtt_request(0);
        let response = AutoDetectResponse::RttResponse {
            sequence_number: req.sequence_number(),
        };
        let _ = mgr.handle_response(&response, 20);

        let snap = mgr.snapshot().expect("one sample recorded");
        match mgr.build_netchar_result().expect("result once samples exist") {
            AutoDetectRequest::NetworkCharacteristicsResult {
                base_rtt_ms,
                bandwidth_kbps,
                average_rtt_ms,
                ..
            } => {
                assert_eq!(base_rtt_ms, Some(snap.min_ms));
                assert_eq!(average_rtt_ms, snap.avg_ms);
                assert_eq!(bandwidth_kbps, None, "RTT-only variant omits bandwidth");
            }
            other => panic!("expected NetworkCharacteristicsResult, got {other:?}"),
        }
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
