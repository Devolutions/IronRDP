//! Fixtures and a driver for the RemoteFX (RFX) decode benchmarks.
//!
//! Realistic RFX bitstreams are built with the encoder, then decoded by the benchmarks.

#![expect(
    clippy::missing_panics_doc,
    reason = "benchmark support code that panics on broken invariants"
)]
#![cfg_attr(
    test,
    expect(unused_crate_dependencies, reason = "rstest is used by integration tests")
)]

mod driver;
mod fixture;
mod pattern;
mod stage;

pub use self::driver::Driver;
pub use self::fixture::{Fixture, fixture};
pub use self::pattern::{Pattern, render};
pub use self::stage::{StageFixture, stage_fixture};

pub const TILE: usize = 64;

/// Offset of the 8x8 LL3 subband, which is last in the 64x64 coefficient layout.
/// Subband reconstruction (the differential decoding step) applies to this slice only.
pub const LL3_OFFSET: usize = TILE * TILE - 8 * 8;

const TILE_SIZE_BYTES: usize = TILE * TILE * 3 /* Y, Cb, Cr planes */;

/// Bytes per pixel of the source and decoded images.
const BPP: usize = 4;

struct Image<'a> {
    width: usize,
    height: usize,
    data: &'a [u8],
}
