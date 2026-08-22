//! Sequence number and timestamp reconstruction.
//!
//! MS-RDPEUDP2 Section 3.1.1.1.3 (sequence numbers) and
//! Section 3.1.1.1.4 (timestamps).
//!
//! All wire-format sequence numbers are 16-bit. Full resolution
//! is 64-bit, reconstructed from a nearby reference value.
//! The active window is limited to `(1 << log_window_size) - 1`
//! which is at most 32767, so 16 bits are sufficient when a
//! reference is available.
//!
//! Timestamps use 24 bits on the wire in units of 4 microseconds,
//! giving a wrap range of ~67 seconds. Reconstruction uses the
//! same principle as sequence numbers but with 24-bit values.

/// Reconstruct a full 64-bit sequence number from a 16-bit wire value
/// and a nearby reference sequence number.
///
/// MS-RDPEUDP2 Section 3.1.1.1.3.
///
/// The algorithm finds the full 64-bit value closest to `reference`
/// whose lower 16 bits match `wire_seq`. This works because the
/// active window is at most 32767 packets, well within 16-bit range.
pub fn reconstruct_seq(wire_seq: u16, reference: u64) -> u64 {
    let wire = u64::from(wire_seq);
    let high = reference & !0xFFFF;
    let candidate = high | wire;

    // Find the candidate closest to the reference.
    // At most three candidates: candidate, candidate-0x10000, candidate+0x10000.
    // The correct one is the one closest to reference.
    if candidate > reference {
        let diff = candidate - reference;
        if diff > 0x8000 && candidate >= 0x10000 {
            candidate - 0x10000
        } else {
            candidate
        }
    } else {
        let diff = reference - candidate;
        if diff > 0x8000 { candidate + 0x10000 } else { candidate }
    }
}

/// Truncate a full 64-bit sequence number to its 16-bit wire representation.
///
/// # Panics
///
/// Infallible: the 0xFFFF mask guarantees the result fits in `u16`.
pub fn truncate_seq(full: u64) -> u16 {
    u16::try_from(full & 0xFFFF).expect("masked to 16 bits fits in u16")
}

/// Reconstruct a full 64-bit timestamp (in 4μs units) from a 24-bit wire value
/// and a nearby reference timestamp.
///
/// MS-RDPEUDP2 Section 3.1.1.1.4.
///
/// Wire timestamps are the lower 24 bits, in units of 4 microseconds.
/// The 24-bit field wraps every `2^24 * 4μs = 67,108,864μs ≈ 67.1 seconds`.
/// A timestamp is considered stale (and should be discarded) if it differs
/// from the current time by more than 32 seconds.
pub fn reconstruct_timestamp(wire_ts: u32, reference: u64) -> u64 {
    let wire = u64::from(wire_ts & TIMESTAMP_24BIT_MASK);
    let high = reference & !u64::from(TIMESTAMP_24BIT_MASK);
    let candidate = high | wire;

    if candidate > reference {
        let diff = candidate - reference;
        if diff > TIMESTAMP_HALF_RANGE && candidate >= TIMESTAMP_WRAP {
            candidate - TIMESTAMP_WRAP
        } else {
            candidate
        }
    } else {
        let diff = reference - candidate;
        if diff > TIMESTAMP_HALF_RANGE {
            candidate + TIMESTAMP_WRAP
        } else {
            candidate
        }
    }
}

/// Truncate a full 64-bit timestamp to its 24-bit wire representation.
///
/// # Panics
///
/// Infallible: the 24-bit mask guarantees the result fits in `u32`.
pub fn truncate_timestamp(full: u64) -> u32 {
    u32::try_from(full & u64::from(TIMESTAMP_24BIT_MASK)).expect("masked to 24 bits fits in u32")
}

/// Convert microseconds to timestamp units (4μs per unit).
pub fn us_to_timestamp_units(us: u64) -> u64 {
    us / TIMESTAMP_UNIT_US
}

/// Convert timestamp units back to microseconds.
pub fn timestamp_units_to_us(units: u64) -> u64 {
    units * TIMESTAMP_UNIT_US
}

/// Mask for 24-bit timestamp values.
const TIMESTAMP_24BIT_MASK: u32 = 0x00FF_FFFF;

/// One timestamp unit = 4 microseconds.
const TIMESTAMP_UNIT_US: u64 = 4;

/// Full wrap range of the 24-bit timestamp field.
const TIMESTAMP_WRAP: u64 = 1 << 24; // 16,777,216 units = 67,108,864μs

/// Half the wrap range, used for closest-value selection.
const TIMESTAMP_HALF_RANGE: u64 = TIMESTAMP_WRAP / 2;

/// Maximum timestamp age before it's considered stale (32 seconds in 4μs units).
/// Per MS-RDPEUDP2 Section 3.1.1.1.4.
pub const TIMESTAMP_STALENESS_LIMIT: u64 = 32_000_000 / TIMESTAMP_UNIT_US; // 8,000,000 units

#[cfg(test)]
mod tests {
    use super::*;

    // ── Sequence number reconstruction ──

    #[test]
    fn reconstruct_seq_same_epoch() {
        // Wire 100, reference 50 → should reconstruct to 100
        assert_eq!(reconstruct_seq(100, 50), 100);
    }

    #[test]
    fn reconstruct_seq_nearby() {
        // Wire 0x0005, reference 0x10003 → should reconstruct to 0x10005
        assert_eq!(reconstruct_seq(0x0005, 0x1_0003), 0x1_0005);
    }

    #[test]
    fn reconstruct_seq_wrap_forward() {
        // Reference near the top of a 16-bit epoch, wire is in the next epoch.
        // Reference = 0x1FFFE, wire = 0x0002 → should reconstruct to 0x20002
        assert_eq!(reconstruct_seq(0x0002, 0x1_FFFE), 0x2_0002);
    }

    #[test]
    fn reconstruct_seq_wrap_backward() {
        // Reference in a new epoch, wire is from the previous epoch.
        // Reference = 0x20002, wire = 0xFFFE → should reconstruct to 0x1FFFE
        assert_eq!(reconstruct_seq(0xFFFE, 0x2_0002), 0x1_FFFE);
    }

    #[test]
    fn reconstruct_seq_exact_match() {
        // Wire matches reference exactly
        assert_eq!(reconstruct_seq(0x1234, 0x5_1234), 0x5_1234);
    }

    #[test]
    fn reconstruct_seq_half_range_forward() {
        // Exactly at the boundary of half range (0x8000)
        assert_eq!(reconstruct_seq(0x8000, 0), 0x8000);
    }

    #[test]
    fn reconstruct_seq_just_past_half_range() {
        // Wire is 0x8001 ahead → should wrap backward
        // Reference = 0, wire = 0x8001
        // candidate = 0x8001, diff = 0x8001 > 0x8000 → wrap back
        // But candidate < 0x10000, so we can't subtract. Edge case.
        // At reference=0, the reconstruction should still handle this.
        let result = reconstruct_seq(0x8001, 0);
        // candidate = 0x8001, diff > 0x8000, candidate >= 0x10000? No.
        // So result = 0x8001.
        assert_eq!(result, 0x8001);
    }

    #[test]
    fn reconstruct_seq_large_reference() {
        // Large reference values
        assert_eq!(reconstruct_seq(0x0042, 0xFFFF_FFFF_0000_0040), 0xFFFF_FFFF_0000_0042);
    }

    #[test]
    fn truncate_seq_basic() {
        assert_eq!(truncate_seq(0x1234_5678), 0x5678);
    }

    #[test]
    fn truncate_seq_max() {
        assert_eq!(truncate_seq(0xFFFF_FFFF_FFFF_FFFF), 0xFFFF);
    }

    #[test]
    fn seq_roundtrip() {
        let original = 0x1_0042u64;
        let wire = truncate_seq(original);
        let reference = 0x1_0040;
        let reconstructed = reconstruct_seq(wire, reference);
        assert_eq!(reconstructed, original);
    }

    #[test]
    fn seq_roundtrip_across_wrap() {
        let original = 0x2_0005u64;
        let wire = truncate_seq(original); // 0x0005
        let reference = 0x1_FFFA;
        let reconstructed = reconstruct_seq(wire, reference);
        assert_eq!(reconstructed, original);
    }

    // ── Timestamp reconstruction ──

    #[test]
    fn reconstruct_ts_same_epoch() {
        assert_eq!(reconstruct_timestamp(1000, 500), 1000);
    }

    #[test]
    fn reconstruct_ts_nearby() {
        let reference = 0x100_0500u64; // well above 24-bit range
        let wire = 0x00_0510u32;
        assert_eq!(reconstruct_timestamp(wire, reference), 0x100_0510);
    }

    #[test]
    fn reconstruct_ts_wrap_forward() {
        // Reference near the end of a 24-bit epoch, wire wraps to next
        let reference = 0x00FF_FFFEu64;
        let wire = 0x00_0002u32;
        assert_eq!(
            reconstruct_timestamp(wire, reference),
            0x0100_0002 // next epoch
        );
    }

    #[test]
    fn reconstruct_ts_wrap_backward() {
        // Reference in new epoch, wire from previous
        let reference = 0x0100_0002u64;
        let wire = 0x00FF_FFFEu32;
        assert_eq!(
            reconstruct_timestamp(wire, reference),
            0x00FF_FFFE // previous epoch
        );
    }

    #[test]
    fn reconstruct_ts_exact_match() {
        assert_eq!(reconstruct_timestamp(0x00_ABCD, 0x500_ABCD), 0x500_ABCD);
    }

    #[test]
    fn truncate_ts_basic() {
        assert_eq!(truncate_timestamp(0xABCD_1234), 0x00CD_1234);
    }

    #[test]
    fn truncate_ts_max_24bit() {
        assert_eq!(truncate_timestamp(0x00FF_FFFF), 0x00FF_FFFF);
    }

    #[test]
    fn ts_roundtrip() {
        let original = 0x100_5000u64;
        let wire = truncate_timestamp(original);
        let reference = 0x100_4FFF;
        let reconstructed = reconstruct_timestamp(wire, reference);
        assert_eq!(reconstructed, original);
    }

    #[test]
    fn ts_roundtrip_across_wrap() {
        let original = 0x200_0010u64;
        let wire = truncate_timestamp(original); // 0x00_0010
        let reference = 0x1FF_FFF0;
        let reconstructed = reconstruct_timestamp(wire, reference);
        assert_eq!(reconstructed, original);
    }

    // ── Timestamp unit conversions ──

    #[test]
    fn us_to_units() {
        assert_eq!(us_to_timestamp_units(0), 0);
        assert_eq!(us_to_timestamp_units(4), 1);
        assert_eq!(us_to_timestamp_units(1_000_000), 250_000); // 1 second
        assert_eq!(us_to_timestamp_units(67_108_864), TIMESTAMP_WRAP); // full 24-bit range
    }

    #[test]
    fn units_to_us() {
        assert_eq!(timestamp_units_to_us(0), 0);
        assert_eq!(timestamp_units_to_us(1), 4);
        assert_eq!(timestamp_units_to_us(250_000), 1_000_000); // 1 second
    }

    #[test]
    fn staleness_limit() {
        // 32 seconds = 32,000,000μs / 4μs = 8,000,000 units
        assert_eq!(TIMESTAMP_STALENESS_LIMIT, 8_000_000);
    }
}
