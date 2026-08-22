//! Loss detection for in-flight packets.
//!
//! MS-RDPEUDP2 Section 3.1.1.2.3.
//!
//! Two independent detection mechanisms:
//!
//! 1. **Reordering-based**: A packet is declared lost if a packet with a
//!    higher DataSeqNum has been acknowledged and the gap exceeds the
//!    reordering threshold.
//!
//! 2. **Time-based**: A packet is declared lost if it has been in flight
//!    longer than the time threshold (typically 1.25 × RTO).
//!
//! When a packet is declared lost, all pending packets with lower
//! DataSeqNum are also declared lost (per spec requirement, since loss is
//! monotonic from the sender's perspective).

use crate::time::MonotonicInstant;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::time::Duration;

use crate::send_window::SendWindow;

/// Default reordering threshold: a packet is lost if at least this many
/// higher-numbered packets have been acknowledged.
///
/// This matches TCP's RACK default and QUIC's kPacketThreshold.
const DEFAULT_REORDER_THRESHOLD: u64 = 3;

/// Time-based loss detection multiplier applied to the RTO.
///
/// A packet is lost if `now - sent_at > time_threshold`.
/// time_threshold = TIME_THRESHOLD_MULTIPLIER × rto.
///
/// 9/8 (1.125) is standard for QUIC; we use 1.25 for slightly more
/// tolerance given RDPEUDP2's higher minimum RTO of 300ms.
const TIME_THRESHOLD_NUMERATOR: u32 = 5;
const TIME_THRESHOLD_DENOMINATOR: u32 = 4;

/// Loss detection state.
#[derive(Debug, Clone)]
pub(crate) struct LossDetector {
    /// Reordering threshold (in packets).
    reorder_threshold: u64,
}

impl LossDetector {
    /// Create a new loss detector with default parameters.
    pub(crate) fn new() -> Self {
        Self {
            reorder_threshold: DEFAULT_REORDER_THRESHOLD,
        }
    }

    /// Compute the time-based loss threshold from the current RTO.
    fn time_threshold(rto: Duration) -> Duration {
        rto * TIME_THRESHOLD_NUMERATOR / TIME_THRESHOLD_DENOMINATOR
    }

    /// Detect newly lost packets in the send window.
    ///
    /// Examines all Pending entries and declares them lost if:
    /// - A higher-numbered packet was ACKed and the gap exceeds
    ///   the reordering threshold, OR
    /// - The packet has been in flight longer than `time_threshold(rto)`.
    ///
    /// Additionally, when a packet is declared lost, all pending packets
    /// with lower DataSeqNum are also declared lost.
    ///
    /// Returns the DataSeqNums of newly detected lost packets.
    pub(crate) fn detect(
        &self,
        send_window: &SendWindow,
        highest_acked: Option<u64>,
        rto: Duration,
        now: MonotonicInstant,
    ) -> Vec<u64> {
        let time_thresh = Self::time_threshold(rto);
        let mut lost_seqs = Vec::new();

        // Collect pending entries that meet loss criteria.
        for entry in send_window.pending_entries() {
            let reorder_lost =
                highest_acked.is_some_and(|ha| ha >= entry.data_seq && (ha - entry.data_seq) >= self.reorder_threshold);

            let time_lost = now.duration_since(entry.sent_at) > time_thresh;

            if reorder_lost || time_lost {
                lost_seqs.push(entry.data_seq);
            }
        }

        if lost_seqs.is_empty() {
            return lost_seqs;
        }

        // Per spec: all pending packets with lower seq than the highest
        // declared-lost packet are also lost.
        let highest_lost = *lost_seqs.iter().max().expect("lost_seqs is non-empty");

        for entry in send_window.pending_entries() {
            if entry.data_seq < highest_lost && !lost_seqs.contains(&entry.data_seq) {
                lost_seqs.push(entry.data_seq);
            }
        }

        // Sort for deterministic ordering.
        lost_seqs.sort_unstable();
        lost_seqs
    }
}

impl Default for LossDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> MonotonicInstant {
        MonotonicInstant::from_millis(0)
    }

    fn later(base: MonotonicInstant, ms: u64) -> MonotonicInstant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn no_loss_when_nothing_pending() {
        let ld = LossDetector::new();
        let w = SendWindow::new(1, 1, 6);
        let rto = Duration::from_millis(300);

        let lost = ld.detect(&w, None, rto, now());
        assert!(lost.is_empty());
    }

    #[test]
    fn no_loss_when_no_acks_and_time_not_expired() {
        let t = now();
        let ld = LossDetector::new();
        let mut w = SendWindow::new(1, 1, 6);
        let rto = Duration::from_millis(300);

        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);

        // No ACKs yet, and time hasn't exceeded threshold
        let lost = ld.detect(&w, None, rto, t);
        assert!(lost.is_empty());
    }

    #[test]
    fn reorder_based_loss() {
        let t = now();
        let ld = LossDetector::new();
        let mut w = SendWindow::new(1, 1, 6);
        let rto = Duration::from_millis(300);

        // Send 5 packets
        for _ in 0..5 {
            w.push(vec![0; 10], t);
        }

        // ACK packets 4 and 5 (gap = 3 from packet 1)
        w.mark_received(4);
        w.mark_received(5);

        // Packet 1 should be lost: highest_acked=5, 5-1=4 >= threshold=3
        // Packet 2 should be lost: 5-2=3 >= threshold=3
        let lost = ld.detect(&w, Some(5), rto, t);
        assert!(lost.contains(&1));
        assert!(lost.contains(&2));
        // Packet 3: 5-3=2 < threshold=3, NOT lost by reorder
        assert!(!lost.contains(&3));
    }

    #[test]
    fn time_based_loss() {
        let t = now();
        let ld = LossDetector::new();
        let mut w = SendWindow::new(1, 1, 6);
        let rto = Duration::from_millis(300);

        w.push(vec![0; 10], t);

        // Time threshold = 300 * 5/4 = 375ms
        // At t+400ms, packet 1 should be lost
        let lost = ld.detect(&w, None, rto, later(t, 400));
        assert_eq!(lost, vec![1]);
    }

    #[test]
    fn time_based_loss_not_triggered_early() {
        let t = now();
        let ld = LossDetector::new();
        let mut w = SendWindow::new(1, 1, 6);
        let rto = Duration::from_millis(300);

        w.push(vec![0; 10], t);

        // Time threshold = 375ms, at t+370ms should NOT be lost
        let lost = ld.detect(&w, None, rto, later(t, 370));
        assert!(lost.is_empty());
    }

    #[test]
    fn lower_packets_implied_lost() {
        let t = now();
        let ld = LossDetector::new();
        let mut w = SendWindow::new(1, 1, 6);
        let rto = Duration::from_millis(300);

        // Send 6 packets
        for _ in 0..6 {
            w.push(vec![0; 10], t);
        }

        // ACK packet 6 only
        w.mark_received(6);

        // Packet 3: 6-3=3 >= threshold → lost
        // Packets 1, 2 have lower seq than 3 → also lost
        let lost = ld.detect(&w, Some(6), rto, t);
        assert!(lost.contains(&1));
        assert!(lost.contains(&2));
        assert!(lost.contains(&3));
        // Packets 4, 5: 6-4=2, 6-5=1, below threshold
        assert!(!lost.contains(&4));
        assert!(!lost.contains(&5));
    }

    #[test]
    fn combined_reorder_and_time_loss() {
        let t = now();
        let ld = LossDetector::new();
        let mut w = SendWindow::new(1, 1, 6);
        let rto = Duration::from_millis(300);

        // Packet 1 sent at t
        w.push(vec![0; 10], t);

        // Packet 2 sent at t+200ms
        w.push(vec![0; 10], later(t, 200));

        // Packet 3, 4, 5 sent at t+350ms
        let t3 = later(t, 350);
        w.push(vec![0; 10], t3);
        w.push(vec![0; 10], t3);
        w.push(vec![0; 10], t3);

        // ACK packet 5
        w.mark_received(5);

        // At t+380ms: time threshold = 375ms
        // Packet 1: sent at t, age = 380ms > 375ms → time-lost
        // Packet 2: sent at t+200, age = 180ms, 5-2=3 >= threshold → reorder-lost
        // Packet 1 also qualifies by reorder: 5-1=4 >= 3
        let lost = ld.detect(&w, Some(5), rto, later(t, 380));
        assert!(lost.contains(&1));
        assert!(lost.contains(&2));
    }

    #[test]
    fn already_lost_packets_not_re_detected() {
        let t = now();
        let ld = LossDetector::new();
        let mut w = SendWindow::new(1, 1, 6);
        let rto = Duration::from_millis(300);

        for _ in 0..5 {
            w.push(vec![0; 10], t);
        }
        w.mark_received(5);

        // First detection
        let lost = ld.detect(&w, Some(5), rto, t);
        assert!(lost.contains(&1));

        // Mark them lost in the window
        for &seq in &lost {
            w.mark_lost(seq);
        }

        // Second detection should find nothing new
        let lost2 = ld.detect(&w, Some(5), rto, t);
        assert!(lost2.is_empty());
    }

    #[test]
    fn time_threshold_computation() {
        let rto = Duration::from_millis(300);
        let thresh = LossDetector::time_threshold(rto);
        assert_eq!(thresh, Duration::from_millis(375));

        let rto = Duration::from_millis(1000);
        let thresh = LossDetector::time_threshold(rto);
        assert_eq!(thresh, Duration::from_millis(1250));
    }

    #[test]
    fn default_matches_new() {
        let from_new = LossDetector::new();
        let from_default = LossDetector::default();
        assert_eq!(from_new.reorder_threshold, from_default.reorder_threshold);
    }
}
