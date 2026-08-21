#[diplomat::bridge]
pub mod ffi {
    /// A reading of the FFI's monotonic clock.
    ///
    /// This is the arrival time a driver attaches to a PDU it just read, and the only value
    /// `Sequence::step` accepts. Differences between two readings are meaningful; the absolute
    /// value is not, since the epoch is an implementation detail.
    ///
    /// All readings come from [`Self::now`], which is backed by a single process-wide clock, so
    /// two instants obtained through this type are always comparable. There is deliberately no
    /// way to build one out of a caller-supplied number: that is how readings from an unrelated
    /// clock would get mixed in.
    #[diplomat::opaque]
    pub struct MonotonicInstant(pub ironrdp::connector::MonotonicInstant);

    impl MonotonicInstant {
        /// Reads the clock now.
        ///
        /// Call this as soon as a read completes, not when the resulting PDU is finally handed to
        /// `step`: the two can be far apart, and only the former is when the bytes arrived.
        pub fn now() -> Box<MonotonicInstant> {
            // Native .NET hosts only, never `wasm32-unknown-unknown`, so `std::time::Instant` is
            // sufficient here (compare `ironrdp-blocking`'s driver clock, same assumption).
            static EPOCH: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
            let elapsed_ms = u64::try_from(EPOCH.elapsed().as_millis()).unwrap_or(u64::MAX);
            Box::new(MonotonicInstant(ironrdp::connector::MonotonicInstant::from_millis(
                elapsed_ms,
            )))
        }

        /// How many milliseconds elapsed between `earlier` and this reading, saturating at zero
        /// if this reading is the earlier of the two.
        pub fn millis_since(&self, earlier: &MonotonicInstant) -> u64 {
            u64::try_from(self.0.duration_since(earlier.0).as_millis()).unwrap_or(u64::MAX)
        }
    }
}
