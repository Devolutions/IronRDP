//! Timer management for the sans-I/O state machine.
//!
//! Per ADR-0001, the connection state machine never calls system clocks.
//! Instead, `MonotonicInstant` values are passed in by the caller and the state
//! machine reports when the next timeout fires via `poll_timeout()`.
//!
//! The `TimerTable` holds a fixed-size array of `Option<MonotonicInstant>` deadlines,
//! one per logical timer. This mirrors Quinn's timer management pattern.

use crate::time::MonotonicInstant;

/// Logical timers used by the RDPEUDP2 connection.
///
/// Each timer has specific semantics described in the variant docs.
/// The numeric values are used as array indices into `TimerTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Timer {
    /// Retransmission timer. Fires when an in-flight packet has not
    /// been acknowledged within the RTO interval.
    ///
    /// Minimum interval: 300ms for RDPEUDP2.
    /// When fired: trigger loss detection and congestion response.
    /// Set when: a packet is sent with no outstanding retransmit timer.
    /// Cleared when: all outstanding packets are acknowledged.
    Retransmit = 0,

    /// ACK delay coalescing timer. Fires when received data packets
    /// have not yet been acknowledged.
    ///
    /// Interval: half the current smoothed RTT, clamped to [50ms, 200ms]
    /// (`RdpeudpConnection::ack_delay_timeout`). [MS-RDPEUDP2] 3.1.5.2 gives
    /// the "half the round trip time" default but no floor or ceiling of
    /// its own; the clamp is [MS-RDPEUDP]'s VERSION_2 analog (3.1.6.3).
    /// When fired: send a standalone ACK.
    /// Set when: a data packet is received and no ACK delay timer is running.
    /// Cleared when: an ACK is sent (piggybacked on data or standalone).
    AckDelay = 1,

    /// Idle timeout. Fires when no packets have been received for the
    /// configured idle timeout duration, indicating the connection is dead.
    ///
    /// Default: 16 seconds (configurable).
    /// When fired: transition to Closed state.
    /// Set when: any packet is received (resets the deadline).
    /// Cleared when: never, only fires or is reset.
    Idle = 2,

    /// Keep-alive timer. Fires when no data has been sent for a while,
    /// triggering a probe packet to prevent the remote's idle timeout
    /// from expiring and to keep NAT mappings alive.
    ///
    /// Default: 8 seconds (should be less than idle timeout).
    /// When fired: send a keep-alive probe (empty data or ACK).
    /// Set when: any packet is sent (resets the deadline).
    /// Cleared when: never, only fires or is reset by sending.
    KeepAlive = 3,
}

impl Timer {
    /// All timer variants, for iteration.
    const ALL: [Self; TIMER_COUNT] = [Self::Retransmit, Self::AckDelay, Self::Idle, Self::KeepAlive];

    /// Array index for this timer variant.
    #[expect(clippy::as_conversions, reason = "repr(u8) enum to usize is safe")]
    const fn index(self) -> usize {
        self as usize
    }
}

/// Number of distinct timer types.
const TIMER_COUNT: usize = 4;

/// Fixed-size table of timer deadlines.
///
/// Each slot is `Option<MonotonicInstant>`: `None` means the timer is inactive,
/// `Some(deadline)` means the timer fires at that instant.
///
/// This design avoids allocation and provides O(1) access to any timer
/// and O(n) scan for the next deadline (where n=4, so effectively O(1)).
#[derive(Debug, Clone)]
pub(crate) struct TimerTable {
    deadlines: [Option<MonotonicInstant>; TIMER_COUNT],
}

impl TimerTable {
    /// Create a new table with all timers inactive.
    pub(crate) fn new() -> Self {
        Self {
            deadlines: [None; TIMER_COUNT],
        }
    }

    /// Set a timer to fire at `deadline`.
    ///
    /// Overwrites any existing deadline for this timer.
    pub(crate) fn set(&mut self, timer: Timer, deadline: MonotonicInstant) {
        self.deadlines[timer.index()] = Some(deadline);
    }

    /// Clear a timer, making it inactive.
    pub(crate) fn clear(&mut self, timer: Timer) {
        self.deadlines[timer.index()] = None;
    }

    /// Get the current deadline for a specific timer, if active.
    #[cfg(test)]
    pub(crate) fn get(&self, timer: Timer) -> Option<MonotonicInstant> {
        self.deadlines[timer.index()]
    }

    /// Whether a specific timer is currently active.
    pub(crate) fn is_set(&self, timer: Timer) -> bool {
        self.deadlines[timer.index()].is_some()
    }

    /// Get the earliest deadline across all active timers.
    ///
    /// Returns `None` if no timers are active.
    /// This is the value the caller should use for `poll_timeout()`.
    pub(crate) fn next_deadline(&self) -> Option<MonotonicInstant> {
        self.deadlines.iter().filter_map(|d| *d).min()
    }

    /// Iterate over timers that have expired as of `now`.
    ///
    /// A timer is expired if its deadline is <= `now`.
    /// Returns an iterator of `Timer` values in enum-order.
    /// The caller should process each expired timer and clear or
    /// reset it as appropriate.
    pub(crate) fn expired(&self, now: MonotonicInstant) -> impl Iterator<Item = Timer> + '_ {
        Timer::ALL
            .iter()
            .copied()
            .filter(move |timer| self.deadlines[timer.index()].is_some_and(|deadline| deadline <= now))
    }
}

impl Default for TimerTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant_at(ms: u64) -> MonotonicInstant {
        MonotonicInstant::from_millis(ms)
    }

    #[test]
    fn new_table_all_inactive() {
        let table = TimerTable::new();
        assert!(!table.is_set(Timer::Retransmit));
        assert!(!table.is_set(Timer::AckDelay));
        assert!(!table.is_set(Timer::Idle));
        assert!(!table.is_set(Timer::KeepAlive));
        assert_eq!(table.next_deadline(), None);
    }

    #[test]
    fn set_and_get() {
        let mut table = TimerTable::new();
        let deadline = instant_at(1000);

        table.set(Timer::Retransmit, deadline);
        assert!(table.is_set(Timer::Retransmit));
        assert_eq!(table.get(Timer::Retransmit), Some(deadline));
    }

    #[test]
    fn clear_timer() {
        let mut table = TimerTable::new();
        table.set(Timer::AckDelay, instant_at(500));
        assert!(table.is_set(Timer::AckDelay));

        table.clear(Timer::AckDelay);
        assert!(!table.is_set(Timer::AckDelay));
        assert_eq!(table.get(Timer::AckDelay), None);
    }

    #[test]
    fn next_deadline_single_timer() {
        let mut table = TimerTable::new();
        let deadline = instant_at(1000);
        table.set(Timer::Idle, deadline);
        assert_eq!(table.next_deadline(), Some(deadline));
    }

    #[test]
    fn next_deadline_returns_earliest() {
        let mut table = TimerTable::new();
        table.set(Timer::Retransmit, instant_at(300));
        table.set(Timer::AckDelay, instant_at(100));
        table.set(Timer::Idle, instant_at(16000));
        table.set(Timer::KeepAlive, instant_at(8000));

        // AckDelay at 100ms is the earliest
        assert_eq!(table.next_deadline(), Some(instant_at(100)));
    }

    #[test]
    fn next_deadline_updates_after_clear() {
        let mut table = TimerTable::new();
        table.set(Timer::AckDelay, instant_at(100));
        table.set(Timer::Retransmit, instant_at(300));

        assert_eq!(table.next_deadline(), Some(instant_at(100)));

        table.clear(Timer::AckDelay);
        assert_eq!(table.next_deadline(), Some(instant_at(300)));
    }

    #[test]
    fn overwrite_deadline() {
        let mut table = TimerTable::new();
        table.set(Timer::Retransmit, instant_at(300));
        assert_eq!(table.get(Timer::Retransmit), Some(instant_at(300)));

        // Overwrite with an earlier deadline
        table.set(Timer::Retransmit, instant_at(150));
        assert_eq!(table.get(Timer::Retransmit), Some(instant_at(150)));
    }

    #[test]
    fn expired_none_when_all_inactive() {
        let table = TimerTable::new();
        let expired: Vec<Timer> = table.expired(instant_at(10000)).collect();
        assert!(expired.is_empty());
    }

    #[test]
    fn expired_none_when_all_in_future() {
        let mut table = TimerTable::new();
        table.set(Timer::Retransmit, instant_at(500));
        table.set(Timer::AckDelay, instant_at(200));

        // Check at t=100, both timers are in the future
        let expired: Vec<Timer> = table.expired(instant_at(100)).collect();
        assert!(expired.is_empty());
    }

    #[test]
    fn expired_returns_past_timers() {
        let mut table = TimerTable::new();
        table.set(Timer::Retransmit, instant_at(300));
        table.set(Timer::AckDelay, instant_at(100));
        table.set(Timer::Idle, instant_at(16000));
        table.set(Timer::KeepAlive, instant_at(8000));

        // At t=500, Retransmit (300) and AckDelay (100) have expired
        let expired: Vec<Timer> = table.expired(instant_at(500)).collect();
        assert_eq!(expired.len(), 2);
        assert!(expired.contains(&Timer::Retransmit));
        assert!(expired.contains(&Timer::AckDelay));
    }

    #[test]
    fn expired_includes_exact_deadline() {
        let mut table = TimerTable::new();
        table.set(Timer::AckDelay, instant_at(100));

        // At exactly the deadline, the timer should be expired
        let expired: Vec<Timer> = table.expired(instant_at(100)).collect();
        assert_eq!(expired, vec![Timer::AckDelay]);
    }

    #[test]
    fn expired_all_timers() {
        let mut table = TimerTable::new();
        table.set(Timer::Retransmit, instant_at(100));
        table.set(Timer::AckDelay, instant_at(200));
        table.set(Timer::Idle, instant_at(300));
        table.set(Timer::KeepAlive, instant_at(400));

        let expired: Vec<Timer> = table.expired(instant_at(500)).collect();
        assert_eq!(expired.len(), 4);
    }

    #[test]
    fn timer_independence() {
        // Setting/clearing one timer doesn't affect others
        let mut table = TimerTable::new();
        table.set(Timer::Retransmit, instant_at(300));
        table.set(Timer::AckDelay, instant_at(100));

        table.clear(Timer::AckDelay);
        assert!(table.is_set(Timer::Retransmit));
        assert_eq!(table.get(Timer::Retransmit), Some(instant_at(300)));
    }

    #[test]
    fn default_matches_new() {
        let from_new = TimerTable::new();
        let from_default = TimerTable::default();
        assert_eq!(from_new.next_deadline(), from_default.next_deadline());
        for timer in Timer::ALL {
            assert_eq!(from_new.get(timer), from_default.get(timer));
        }
    }

    #[test]
    fn expired_preserves_enum_order() {
        let mut table = TimerTable::new();
        // Set all with same deadline
        for timer in Timer::ALL {
            table.set(timer, instant_at(100));
        }

        let expired: Vec<Timer> = table.expired(instant_at(200)).collect();
        // Should come in enum order: Retransmit, AckDelay, Idle, KeepAlive
        assert_eq!(
            expired,
            vec![Timer::Retransmit, Timer::AckDelay, Timer::Idle, Timer::KeepAlive]
        );
    }
}
