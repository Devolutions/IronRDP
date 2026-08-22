//! NewReno congestion control for RDPEUDP2.
//!
//! MS-RDPEUDP Section 3.1.1 and MS-RDPEUDP2 Section 3.1.1.2.
//!
//! Implements a standard NewReno-style congestion controller with slow
//! start and congestion avoidance phases.
//!
//! Loss is the only congestion signal. MS-RDPEUDP2 has no explicit one:
//! 3.1.1.2.2 names a Congestion Controller as a higher-layer concept for
//! "inferring the runtime network conditions" and its packet header
//! (2.2.1.1) carries nothing but payload-presence flags. The CN and CWR
//! flags belong to MS-RDPEUDP, whose RDPUDP_FEC_HEADER defines them, and
//! this crate speaks the v2 data transfer.
//!
//! At most one reaction per recovery epoch; see `on_loss`.

/// Initial congestion window size in bytes.
///
/// MS-RDPEUDP2 doesn't specify an initial window. Quinn uses 14720
/// bytes (10 * 1200 + spare). We use 10 * 1232 = 12320, aligned to
/// the RDPEUDP2 MTU of 1232 bytes.
const INITIAL_WINDOW: u64 = 10 * 1232;

/// Minimum congestion window: 2 × MTU.
///
/// The window never drops below this, even after loss events.
const MIN_WINDOW: u64 = 2 * 1232;

/// NewReno congestion controller.
///
/// Manages the congestion window (cwnd), slow start threshold (ssthresh),
/// and loss epoch tracking via the CWR sequence number.
#[derive(Debug, Clone)]
pub(crate) struct CongestionControl {
    /// Congestion window in bytes.
    window: u64,

    /// Slow start threshold in bytes.
    /// Set to u64::MAX initially (all traffic starts in slow start).
    ssthresh: u64,

    /// Bytes acknowledged since the last window increase.
    /// Used for congestion avoidance: increase by 1 MTU per cwnd bytes ACKed.
    bytes_acked: u64,

    /// Highest DataSeqNum in flight when the window was last reduced by a
    /// locally detected loss.
    ///
    /// Everything at or below it was already on the wire when the reduction
    /// happened, so losing it is part of the same event and must not reduce
    /// the window again.
    recovery_seq: Option<u64>,
}

impl CongestionControl {
    /// Create a new controller with the default initial window.
    pub(crate) fn new() -> Self {
        Self::with_initial_window(INITIAL_WINDOW)
    }

    /// Create a controller with a specified initial window size.
    pub(crate) fn with_initial_window(initial_window: u64) -> Self {
        Self {
            window: initial_window,
            ssthresh: u64::MAX,
            bytes_acked: 0,
            recovery_seq: None,
        }
    }

    /// Current congestion window in bytes.
    ///
    /// The caller should ensure `bytes_in_flight <= window()` before
    /// transmitting new data.
    pub(crate) fn window(&self) -> u64 {
        self.window
    }

    /// Whether the controller is in slow start phase.
    pub(crate) fn in_slow_start(&self) -> bool {
        self.window < self.ssthresh
    }

    /// Current slow start threshold.
    #[cfg(test)]
    pub(crate) fn ssthresh(&self) -> u64 {
        self.ssthresh
    }

    /// Called when an ACK acknowledges new data.
    ///
    /// In slow start: window increases by `newly_acked_bytes` (doubles
    /// roughly every RTT).
    /// In congestion avoidance: window increases by approximately
    /// 1 MTU per cwnd bytes ACKed (linear increase).
    pub(crate) fn on_ack(&mut self, newly_acked_bytes: u64) {
        if self.in_slow_start() {
            // Slow start: exponential growth
            self.window += newly_acked_bytes;

            // Don't overshoot ssthresh
            if self.window >= self.ssthresh {
                self.bytes_acked = 0;
            }
        } else {
            // Congestion avoidance: linear growth
            // Increase by 1 MTU per cwnd bytes acknowledged
            self.bytes_acked += newly_acked_bytes;

            if self.bytes_acked >= self.window {
                self.bytes_acked -= self.window;
                self.window += 1232; // 1 MTU increase
            }
        }
    }

    /// Called when a packet is declared lost.
    ///
    /// `loss_seq` is the DataSeqNum of the lost packet and `largest_sent`
    /// the highest DataSeqNum handed to the network so far.
    ///
    /// The window is reduced once per recovery epoch, not once per lost
    /// packet. A burst of loss is one congestion event: everything already
    /// in flight when the reduction happened belongs to it, so only losing
    /// something sent afterwards starts a new epoch. Halving per packet
    /// instead puts the window on its floor after a single burst, and the
    /// floor is two MTUs.
    ///
    /// Returns `true` if the window was actually reduced.
    pub(crate) fn on_loss(&mut self, loss_seq: u64, largest_sent: u64) -> bool {
        if let Some(recovery) = self.recovery_seq {
            if loss_seq <= recovery {
                // Same congestion event: already reacted.
                return false;
            }
        }

        self.recovery_seq = Some(largest_sent);
        self.halve_window();
        true
    }

    /// Halve the window and set the slow-start threshold to match.
    fn halve_window(&mut self) {
        self.ssthresh = (self.window / 2).max(MIN_WINDOW);
        self.window = self.ssthresh;
        self.bytes_acked = 0;
    }
}

impl Default for CongestionControl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state() {
        let cc = CongestionControl::new();
        assert_eq!(cc.window(), INITIAL_WINDOW);
        assert!(cc.in_slow_start()); // ssthresh = MAX
    }

    #[test]
    fn slow_start_doubles_per_rtt() {
        let mut cc = CongestionControl::new();
        let initial = cc.window();

        // ACK the entire window → window should roughly double
        cc.on_ack(initial);

        assert_eq!(cc.window(), initial * 2);
        assert!(cc.in_slow_start());
    }

    #[test]
    fn slow_start_growth() {
        let mut cc = CongestionControl::new();

        cc.on_ack(1232); // 1 MTU acked
        assert_eq!(cc.window(), INITIAL_WINDOW + 1232);
    }

    #[test]
    fn transition_to_congestion_avoidance_on_loss() {
        let mut cc = CongestionControl::new();
        let initial = cc.window();

        // Loss event
        cc.on_loss(1, 1);
        assert!(!cc.in_slow_start()); // ssthresh = window/2 = initial/2, window = ssthresh
        assert_eq!(cc.window(), initial / 2);
        assert_eq!(cc.ssthresh(), initial / 2);
    }

    #[test]
    fn congestion_avoidance_linear_growth() {
        let mut cc = CongestionControl::with_initial_window(12320);

        // Force into congestion avoidance
        cc.on_loss(1, 1);
        let window_after_loss = cc.window(); // 6160

        // In congestion avoidance, need to ACK ~cwnd bytes to gain 1 MTU
        cc.on_ack(window_after_loss);

        // Window should increase by 1 MTU
        assert_eq!(cc.window(), window_after_loss + 1232);
    }

    #[test]
    fn window_floor_on_loss() {
        let mut cc = CongestionControl::with_initial_window(MIN_WINDOW);

        cc.on_loss(1, 1);
        // Window should not drop below MIN_WINDOW
        assert_eq!(cc.window(), MIN_WINDOW);
    }

    #[test]
    fn on_loss_once_per_epoch() {
        let mut cc = CongestionControl::new();
        let initial = cc.window();

        // First loss, with sequence numbers up to 10 already in flight.
        assert!(cc.on_loss(5, 10));
        let after_first = cc.window();
        assert_eq!(after_first, initial / 2);

        // Another packet from that same flight: same event, no reaction.
        assert!(!cc.on_loss(7, 14));
        assert_eq!(cc.window(), after_first);

        // Something sent after the reduction: a new event.
        assert!(cc.on_loss(12, 20));
        assert_eq!(cc.window(), after_first / 2);
    }

    /// Losing a burst is one congestion event, not one per packet. Reducing
    /// per packet puts the window on its floor from a single burst.
    #[test]
    fn a_burst_of_loss_reduces_the_window_once() {
        let mut cc = CongestionControl::new();
        let initial = cc.window();

        // Ten packets, sequence numbers 1 to 10, all declared lost together.
        for loss_seq in 1..=10 {
            cc.on_loss(loss_seq, 10);
        }

        assert_eq!(cc.window(), initial / 2);
        assert!(cc.window() > MIN_WINDOW, "the window collapsed to its floor");
    }

    #[test]
    fn repeated_loss_converges_to_floor() {
        let mut cc = CongestionControl::new();

        // Each loss is of a packet sent after the previous reduction, so
        // each one is its own congestion event.
        for seq in 0..50 {
            cc.on_loss(seq, seq);
        }

        assert_eq!(cc.window(), MIN_WINDOW);
    }

    #[test]
    fn default_matches_new() {
        let from_new = CongestionControl::new();
        let from_default = CongestionControl::default();
        assert_eq!(from_new.window(), from_default.window());
        assert_eq!(from_new.ssthresh(), from_default.ssthresh());
    }

    #[test]
    fn custom_initial_window() {
        let cc = CongestionControl::with_initial_window(5000);
        assert_eq!(cc.window(), 5000);
    }

    #[test]
    fn slow_start_to_avoidance_boundary() {
        let mut cc = CongestionControl::new();
        let initial = cc.window();

        // Trigger loss to set ssthresh
        cc.on_loss(1, 1);
        let ssthresh = cc.ssthresh();
        assert_eq!(ssthresh, initial / 2);

        // Window is at ssthresh, so we're in congestion avoidance
        assert!(!cc.in_slow_start());

        // If window somehow goes below ssthresh, we'd be in slow start
        // (this doesn't happen normally, but tests the boundary)
    }
}
