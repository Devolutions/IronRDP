//! Monotonic time readings supplied by the caller.
//!
//! The connection state machine never reads a clock. It receives the current
//! instant on every call that can advance time, and reports when it next wants
//! to be woken through [`RdpeudpConnection::poll_timeout`].
//!
//! [`MonotonicInstant`] itself lives in `ironrdp-sequence`, shared with
//! `ironrdp-connector`'s `Sequence::step`, and is re-exported here so
//! existing callers of this crate are unaffected.
//!
//! [`RdpeudpConnection::poll_timeout`]: crate::RdpeudpConnection::poll_timeout

pub use ironrdp_sequence::MonotonicInstant;
