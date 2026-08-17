//! RTT estimation and RTO calculation.
//!
//! Implements RFC 6298 ("Computing TCP's Retransmission Timer") adapted
//! for RDPEUDP2. The key difference from standard TCP: the minimum RTO
//! is 300ms, not TCP's 1 second. MS-RDPEUDP2 Section 3.1.1.2.3 itself only
//! says the loss-detection timeout is "dynamically variable, depending on
//! the current network conditions", with no fixed floor of its own; the
//! 300ms figure is [MS-RDPEUDP] (the v1 document) Section 3.1.6.1's
//! VERSION_2 minimum retransmit timeout, used here as the closest available
//! spec anchor.
//!
//! RTT samples are derived from the ACK payload's `receivedTS` and
//! `sendAckTimeGap` fields. Samples from delayed ACKs (ACKDELAYED flag)
//! must be excluded by the caller, as they would inflate the estimate.

use core::time::Duration;

/// Minimum RTO for RDPEUDP2 connections.
///
/// [MS-RDPEUDP] Section 3.1.6.1's VERSION_2 minimum retransmit timeout
/// (see the module doc comment for why that document rather than
/// MS-RDPEUDP2 itself). This is lower than TCP's 1s minimum (RFC 6298
/// Section 2.4) because the protocol operates over a typically low-latency
/// network path (RDP sessions).
const MIN_RTO: Duration = Duration::from_millis(300);

/// Maximum RTO cap to prevent unbounded backoff.
///
/// RFC 6298 Section 2.5 recommends at least 60 seconds.
/// Quinn uses 16 seconds. We use 16 seconds to match Quinn's behavior,
/// which is reasonable for interactive RDP sessions where longer
/// timeouts would degrade user experience beyond recovery.
const MAX_RTO: Duration = Duration::from_secs(16);

/// Initial RTO before any RTT measurement is available.
///
/// RFC 6298 Section 2.1 recommends 1 second as the initial value.
/// RDPEUDP2 doesn't specify an override, so we use the RFC default.
const INITIAL_RTO: Duration = Duration::from_secs(1);

/// RFC 6298 smoothing factor alpha = 1/8.
const SRTT_ALPHA: u32 = 8;

/// RFC 6298 deviation factor beta = 1/4.
const RTTVAR_BETA: u32 = 4;

/// RTT estimator with smoothed RTT, RTT variance, and RTO calculation.
///
/// Follows RFC 6298 Section 2 with the RDPEUDP2 minimum RTO of 300ms.
///
/// # Usage
///
/// The caller provides RTT samples from ACK processing. Each sample
/// is the measured round-trip time for a specific packet. The estimator
/// maintains a smoothed RTT (SRTT) and variance (RTTVAR) to compute
/// the retransmission timeout (RTO).
///
/// ```ignore
/// let mut rtt = RttEstimator::new();
/// // First sample initializes SRTT and RTTVAR
/// rtt.update(Duration::from_millis(50));
/// assert!(rtt.rto() >= Duration::from_millis(300)); // min RTO enforced
///
/// // Subsequent samples refine the estimate
/// rtt.update(Duration::from_millis(48));
/// rtt.update(Duration::from_millis(52));
/// ```
#[derive(Debug, Clone)]
pub(crate) struct RttEstimator {
    /// Smoothed RTT. `None` until the first sample is processed.
    srtt: Option<Duration>,

    /// RTT variance (mean deviation).
    /// Initialized to half the first RTT sample per RFC 6298 Section 2.2.
    rttvar: Duration,

    /// Current retransmission timeout, always clamped to [MIN_RTO, MAX_RTO].
    rto: Duration,
}

impl RttEstimator {
    /// Create a new estimator with no RTT samples.
    ///
    /// The initial RTO is 1 second per RFC 6298 Section 2.1.
    pub(crate) fn new() -> Self {
        Self {
            srtt: None,
            rttvar: Duration::ZERO,
            rto: INITIAL_RTO,
        }
    }

    /// Update the estimate with a new RTT sample.
    ///
    /// Implements RFC 6298 Section 2.2 (first sample) and Section 2.3
    /// (subsequent samples).
    ///
    /// The caller must not pass samples from delayed ACKs (those with
    /// the ACKDELAYED flag set), as they would inflate the estimate.
    /// The caller must also not pass samples from retransmitted packets,
    /// per Karn's algorithm (RFC 6298 Section 3).
    pub(crate) fn update(&mut self, rtt: Duration) {
        match self.srtt {
            None => {
                // RFC 6298 Section 2.2: First measurement
                // SRTT = R
                // RTTVAR = R/2
                // RTO = SRTT + max(G, K*RTTVAR) where K=4, G=clock granularity
                // We treat G as negligible (sub-microsecond system clocks).
                self.srtt = Some(rtt);
                self.rttvar = rtt / 2;
            }
            Some(srtt) => {
                // RFC 6298 Section 2.3: Subsequent measurements
                // RTTVAR = (1 - beta) * RTTVAR + beta * |SRTT - R|
                //        = 3/4 * RTTVAR + 1/4 * |SRTT - R|
                // SRTT = (1 - alpha) * SRTT + alpha * R
                //      = 7/8 * SRTT + 1/8 * R

                let abs_diff = srtt.abs_diff(rtt);

                // RTTVAR update must happen BEFORE SRTT update (RFC 6298 Section 2.3)
                self.rttvar = self.rttvar.mul_f64(1.0 - 1.0 / f64::from(RTTVAR_BETA)) + abs_diff / RTTVAR_BETA;

                self.srtt = Some(srtt.mul_f64(1.0 - 1.0 / f64::from(SRTT_ALPHA)) + rtt / SRTT_ALPHA);
            }
        }

        self.recompute_rto();
    }

    /// Current retransmission timeout.
    ///
    /// Always in the range [300ms, 16s] for RDPEUDP2.
    pub(crate) fn rto(&self) -> Duration {
        self.rto
    }

    /// Current smoothed RTT, if any samples have been collected.
    pub(crate) fn srtt(&self) -> Option<Duration> {
        self.srtt
    }

    /// Current RTT variance.
    #[cfg(test)]
    pub(crate) fn rttvar(&self) -> Duration {
        self.rttvar
    }

    /// Apply exponential backoff to the RTO after a retransmission timeout.
    ///
    /// RFC 6298 Section 5.5: double the RTO on each successive timeout
    /// for the same segment, capped at MAX_RTO.
    pub(crate) fn on_timeout(&mut self) {
        self.rto = (self.rto * 2).min(MAX_RTO);
    }

    /// Recompute RTO from SRTT and RTTVAR, applying floor and ceiling.
    fn recompute_rto(&mut self) {
        if let Some(srtt) = self.srtt {
            // RFC 6298: RTO = SRTT + max(G, K*RTTVAR) where K=4
            // G (clock granularity) treated as negligible
            let k_rttvar = self.rttvar * 4;

            // Ensure RTO is at least SRTT + some variance
            // When RTTVAR is very small, use a 1ms floor for the variance component
            let variance_floor = Duration::from_millis(1);
            let rto = srtt + k_rttvar.max(variance_floor);

            self.rto = rto.clamp(MIN_RTO, MAX_RTO);
        }
        // If no SRTT yet, keep the initial RTO
    }
}

impl Default for RttEstimator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state() {
        let rtt = RttEstimator::new();
        assert_eq!(rtt.srtt(), None);
        assert_eq!(rtt.rttvar(), Duration::ZERO);
        assert_eq!(rtt.rto(), INITIAL_RTO);
    }

    #[test]
    fn first_sample_initializes() {
        let mut rtt = RttEstimator::new();
        let sample = Duration::from_millis(100);

        rtt.update(sample);

        // RFC 6298 Section 2.2: SRTT = R, RTTVAR = R/2
        assert_eq!(rtt.srtt(), Some(sample));
        assert_eq!(rtt.rttvar(), sample / 2);
    }

    #[test]
    fn first_sample_rto_floor() {
        let mut rtt = RttEstimator::new();
        // Very small RTT → RTO should be clamped to 300ms minimum
        rtt.update(Duration::from_millis(10));

        assert_eq!(rtt.srtt(), Some(Duration::from_millis(10)));
        assert_eq!(rtt.rto(), MIN_RTO);
    }

    #[test]
    fn first_sample_rto_computation() {
        let mut rtt = RttEstimator::new();
        // R = 200ms → SRTT = 200ms, RTTVAR = 100ms
        // RTO = SRTT + 4*RTTVAR = 200 + 400 = 600ms
        rtt.update(Duration::from_millis(200));

        assert_eq!(rtt.srtt(), Some(Duration::from_millis(200)));
        assert_eq!(rtt.rttvar(), Duration::from_millis(100));
        assert_eq!(rtt.rto(), Duration::from_millis(600));
    }

    #[test]
    fn subsequent_sample_stable() {
        let mut rtt = RttEstimator::new();
        // Initialize with 100ms
        rtt.update(Duration::from_millis(100));

        // Second sample identical → estimate should converge
        rtt.update(Duration::from_millis(100));

        // RTTVAR = 3/4 * 50 + 1/4 * |100 - 100| = 37.5ms
        // SRTT = 7/8 * 100 + 1/8 * 100 = 100ms
        // RTO = 100 + 4 * 37.5 = 250ms → clamped to 300ms
        assert_eq!(rtt.srtt(), Some(Duration::from_millis(100)));
        assert_eq!(rtt.rto(), MIN_RTO);
    }

    #[test]
    fn subsequent_sample_increase() {
        let mut rtt = RttEstimator::new();
        rtt.update(Duration::from_millis(100));
        rtt.update(Duration::from_millis(200));

        // RTTVAR = 3/4 * 50 + 1/4 * |100 - 200| = 37.5 + 25 = 62.5ms
        // SRTT = 7/8 * 100 + 1/8 * 200 = 87.5 + 25 = 112.5ms
        // RTO = 112.5 + 4 * 62.5 = 362.5ms
        let srtt = rtt.srtt().expect("should have SRTT");
        assert!(srtt > Duration::from_millis(110) && srtt < Duration::from_millis(115));

        let rto = rtt.rto();
        assert!(rto > Duration::from_millis(360) && rto < Duration::from_millis(365));
    }

    #[test]
    fn convergence_with_stable_rtt() {
        let mut est = RttEstimator::new();
        // Feed 20 identical samples of 80ms
        for _ in 0..20 {
            est.update(Duration::from_millis(80));
        }

        // SRTT should be very close to 80ms
        let srtt = est.srtt().expect("should have SRTT");
        let srtt_ms = srtt.as_micros();
        assert!(
            (79_500..=80_500).contains(&srtt_ms),
            "expected SRTT near 80ms, got {srtt_ms}μs"
        );

        // RTTVAR should be very small (near zero variance)
        let rttvar_us = est.rttvar().as_micros();
        assert!(rttvar_us < 5_000, "expected small RTTVAR, got {rttvar_us}μs");
    }

    #[test]
    fn rto_min_floor_enforced() {
        let mut est = RttEstimator::new();
        // Extremely fast connection → RTO should still be >= 300ms
        est.update(Duration::from_micros(500));
        assert!(
            est.rto() >= MIN_RTO,
            "RTO {} < min {}",
            est.rto().as_millis(),
            MIN_RTO.as_millis()
        );

        est.update(Duration::from_micros(500));
        assert!(est.rto() >= MIN_RTO);
    }

    #[test]
    fn rto_max_ceiling_enforced() {
        let mut est = RttEstimator::new();
        // Very high RTT → RTO should be capped
        est.update(Duration::from_secs(10));

        // SRTT=10s, RTTVAR=5s → RTO = 10 + 4*5 = 30s → capped at 16s
        assert_eq!(est.rto(), MAX_RTO);
    }

    #[test]
    fn timeout_backoff() {
        let mut est = RttEstimator::new();
        est.update(Duration::from_millis(200));

        let rto_before = est.rto();
        est.on_timeout();
        let rto_after = est.rto();

        // RTO should double
        assert_eq!(rto_after, rto_before * 2);
    }

    #[test]
    fn timeout_backoff_respects_max() {
        let mut est = RttEstimator::new();
        est.update(Duration::from_millis(200));

        // Double repeatedly until we hit the ceiling
        for _ in 0..20 {
            est.on_timeout();
        }

        assert_eq!(est.rto(), MAX_RTO);
    }

    #[test]
    fn timeout_backoff_then_sample_resets() {
        let mut est = RttEstimator::new();
        est.update(Duration::from_millis(100));

        // Back off several times
        est.on_timeout();
        est.on_timeout();
        let backed_off_rto = est.rto();
        assert!(backed_off_rto > Duration::from_secs(1));

        // A new RTT sample recalculates RTO from SRTT/RTTVAR,
        // effectively resetting from the backoff
        est.update(Duration::from_millis(100));
        assert!(est.rto() < backed_off_rto);
    }

    #[test]
    fn default_matches_new() {
        let from_new = RttEstimator::new();
        let from_default = RttEstimator::default();
        assert_eq!(from_new.rto(), from_default.rto());
        assert_eq!(from_new.srtt(), from_default.srtt());
        assert_eq!(from_new.rttvar(), from_default.rttvar());
    }

    #[test]
    fn jittery_samples() {
        let mut est = RttEstimator::new();
        // Simulate a jittery connection: RTT oscillates between 40ms and 120ms
        let samples = [40, 120, 45, 115, 42, 118, 48, 112, 40, 120];
        for &ms in &samples {
            est.update(Duration::from_millis(ms));
        }

        // SRTT should be near the mean (~80ms)
        let srtt = est.srtt().expect("should have SRTT");
        let srtt_ms = srtt.as_millis();
        assert!(
            (60..=100).contains(&srtt_ms),
            "expected SRTT near 80ms, got {srtt_ms}ms"
        );

        // RTTVAR should be non-trivial due to jitter
        let rttvar_ms = est.rttvar().as_millis();
        assert!(rttvar_ms > 5, "expected non-trivial RTTVAR, got {rttvar_ms}ms");
    }

    #[test]
    fn decreasing_rtt_adjusts_downward() {
        let mut est = RttEstimator::new();
        // Start high, then decrease
        est.update(Duration::from_millis(500));
        let initial_rto = est.rto();

        for _ in 0..10 {
            est.update(Duration::from_millis(50));
        }

        // RTO should have decreased substantially
        assert!(est.rto() < initial_rto);
    }

    #[test]
    fn rttvar_ordering_in_update() {
        // Verify that RTTVAR is updated before SRTT per RFC 6298 Section 2.3.
        // The order matters because RTTVAR uses the current (old) SRTT,
        // not the newly computed one.
        let mut est = RttEstimator::new();
        est.update(Duration::from_millis(100));

        // Large spike
        est.update(Duration::from_millis(500));

        // RTTVAR should reflect |old_SRTT(100) - R(500)| = 400,
        // contributing 1/4 * 400 = 100ms to the new RTTVAR.
        // Previous RTTVAR was 50ms, so new = 3/4 * 50 + 1/4 * 400 = 37.5 + 100 = 137.5ms
        let rttvar_ms = est.rttvar().as_millis();
        assert!(
            (135..=140).contains(&rttvar_ms),
            "expected RTTVAR ~137.5ms, got {rttvar_ms}ms"
        );
    }
}
