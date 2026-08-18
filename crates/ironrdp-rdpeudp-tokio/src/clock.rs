//! The clock the sans-I/O state machine deliberately does not have.
//!
//! [`ironrdp_rdpeudp::RdpeudpConnection`] never reads a clock. It takes a
//! [`MonotonicInstant`] on every call that can advance time, and reports the
//! deadline it next wants through `poll_timeout`. This module is the only place
//! in the transport that calls [`Instant::now`], and it translates in both
//! directions across that boundary.

use core::time::Duration;
use std::time::Instant;

use ironrdp_rdpeudp::MonotonicInstant;

/// Translates between the driver's [`Instant`] readings and the millisecond
/// instants the state machine understands.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Clock {
    epoch: Instant,
}

impl Clock {
    /// Starts a clock whose epoch is now.
    pub(crate) fn new() -> Self {
        Self { epoch: Instant::now() }
    }

    /// The current reading.
    pub(crate) fn now(&self) -> MonotonicInstant {
        self.reading(Instant::now())
    }

    /// The reading for an instant the driver already observed, such as the
    /// moment a datagram came off the socket.
    pub(crate) fn reading(&self, instant: Instant) -> MonotonicInstant {
        let elapsed = instant.saturating_duration_since(self.epoch).as_millis();
        MonotonicInstant::from_millis(u64::try_from(elapsed).unwrap_or(u64::MAX))
    }

    /// The [`Instant`] a state-machine deadline corresponds to, for handing to
    /// `tokio::time::sleep_until`.
    pub(crate) fn deadline(&self, instant: MonotonicInstant) -> Instant {
        self.epoch
            .checked_add(Duration::from_millis(instant.as_millis()))
            .unwrap_or(self.epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_advance_with_the_epoch() {
        let clock = Clock::new();
        let epoch = clock.epoch;

        assert_eq!(clock.reading(epoch), MonotonicInstant::from_millis(0));
        assert_eq!(
            clock.reading(epoch + Duration::from_millis(250)),
            MonotonicInstant::from_millis(250)
        );
    }

    #[test]
    fn readings_saturate_before_the_epoch() {
        let clock = Clock::new();
        let before = clock.epoch - Duration::from_secs(1);

        assert_eq!(clock.reading(before), MonotonicInstant::from_millis(0));
    }

    #[test]
    fn deadlines_round_trip_through_readings() {
        let clock = Clock::new();
        let instant = MonotonicInstant::from_millis(1_500);

        assert_eq!(clock.reading(clock.deadline(instant)), instant);
    }
}
