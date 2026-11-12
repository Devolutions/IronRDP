//! RemoteFX decode benchmarks.
//!
//! Benchmarks are grouped by granularity. From lowest to highest:
//!
//! * `stage/*` -- each decode stage on its own, to attribute the frame time.
//! * `tile/*` -- one 64x64 tile, to compare per-tile cost across patterns.
//! * `frame/*` -- a full-screen frame through `rfx::DecodingContext::decode`.
//!
//! `entropy/*` is a separate comparison: the same full-screen frame decoded
//! with RLGR1 and RLGR3. All other groups use RLGR3.
//!
//! Throughput is reported in pixels, so results are comparable across
//! resolutions and directly convertible to frames per second.

#![expect(unused_crate_dependencies, reason = "not all bench targets use all dependencies")]

mod entropy;
mod frame;
mod stage;
mod tile;

use criterion::{criterion_group, criterion_main};
use ironrdp::pdu::codecs::rfx::EntropyAlgorithm;

/// IronRDP advertises RLGR3 in RemoteFX client caps.
const ENTROPY_ALGORITHM: EntropyAlgorithm = EntropyAlgorithm::Rlgr3;

criterion_group!(benches, frame::frame, tile::tile, entropy::entropy, stage::stage);
criterion_main!(benches);
