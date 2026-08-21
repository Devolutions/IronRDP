//! Monotonic time readings supplied by the caller.
//!
//! Sans-I/O state machines never read a clock themselves. They receive the
//! current instant on every call that can advance time, and report when they
//! next want to be woken. The clock deliberately lives outside the sequence
//! or connection type, because a state machine reading a clock itself would
//! measure how quickly it drained an already-filled buffer rather than when
//! the bytes actually arrived on the wire; only the I/O driver that performed
//! the read knows that.
//!
//! `std::time::Instant` is deliberately not used here. `Instant::now` panics
//! on `wasm32-unknown-unknown`, which some drivers using this type compile
//! for, and this crate is `no_std` besides.
//!
//! This type needs no allocation and stays available even when the
//! `state-machine` feature is disabled; see the crate's README for the full
//! feature-split rationale.

use core::ops::Add;
use core::time::Duration;

/// A monotonic instant, in milliseconds, from an epoch chosen by the caller.
///
/// The epoch is arbitrary and carries no meaning; only differences between two
/// instants do. Millisecond resolution is well below the shortest interval any
/// current caller needs to measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonotonicInstant(u64);

impl MonotonicInstant {
    /// Builds an instant from a monotonic millisecond reading.
    #[must_use]
    pub fn from_millis(milliseconds: u64) -> Self {
        Self(milliseconds)
    }

    /// The reading this instant was built from.
    #[must_use]
    pub fn as_millis(self) -> u64 {
        self.0
    }

    /// The time elapsed since `earlier`, saturating at zero if the clock went
    /// backwards or the arguments were transposed.
    #[must_use]
    pub fn duration_since(self, earlier: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }

    /// This instant advanced by `duration`, saturating at the end of the
    /// representable range.
    ///
    /// Saturating is the right behavior for the only caller: a deadline that
    /// cannot be represented is one that never fires, which is what a timer set
    /// beyond the end of time should do.
    #[must_use]
    pub fn saturating_add(self, duration: Duration) -> Self {
        let milliseconds = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        Self(self.0.saturating_add(milliseconds))
    }
}

impl Add<Duration> for MonotonicInstant {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self {
        self.saturating_add(rhs)
    }
}
