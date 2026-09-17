//! RemoteFX decode benchmarks.
//!
//! Benchmarks are broken up into three levels. From lowest to highest:
//!
//! * `stage/*` -- each decode stage on its own, to attribute the frame time.
//! * `tile/*` -- one 64x64 tile, to compare per-tile cost across patterns.
//! * `frame/*` -- a full-screen frame through `rfx::DecodingContext::decode`.
//!
//! Throughput is reported in pixels, so results are comparable across
//! resolutions and directly convertible to frames per second.

#![allow(unused_crate_dependencies)] // Not every dependency is used by every target.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ironrdp_graphics::color_conversion::{YCbCrBuffer, ycbcr_to_rgba};
use ironrdp_graphics::{dwt, quantization, rlgr, subband_reconstruction};
use ironrdp_pdu::codecs::rfx::EntropyAlgorithm;
use ironrdp_rfxbench::{Driver, Pattern, TILE, fixture, stage_fixture};

/// IronRDP advertise RLGR3 in RemoteFX client caps.
const ALGO: EntropyAlgorithm = EntropyAlgorithm::Rlgr3;

/// Full-screen decode at common screen resolutions.
fn frame(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame");

    for (w, h) in [(1280u16, 720u16), (1920, 1080), (2560, 1440)] {
        for pattern in Pattern::ALL {
            let fx = fixture(pattern, w, h, ALGO);
            let mut driver = Driver::new(&fx);

            group.throughput(Throughput::Elements(fx.pixels()));
            group.bench_function(BenchmarkId::new(format!("{w}x{h}"), pattern.name()), |b| {
                b.iter(|| driver.decode(&fx.frame))
            });
        }
    }

    group.finish();
}

/// One 64x64 tile, isolating per-tile cost from the framebuffer blit's cache behaviour.
fn tile(c: &mut Criterion) {
    let mut group = c.benchmark_group("tile");

    for pattern in Pattern::ALL {
        let fx = fixture(pattern, 64, 64, ALGO);
        let mut driver = Driver::new(&fx);

        group.throughput(Throughput::Elements(fx.pixels()));
        group.bench_function(pattern.name(), |b| b.iter(|| driver.decode(&fx.frame)));
    }

    group.finish();
}

/// Both entropy coders.
fn entropy(c: &mut Criterion) {
    let mut group = c.benchmark_group("entropy");

    for algorithm in [EntropyAlgorithm::Rlgr1, EntropyAlgorithm::Rlgr3] {
        let fx = fixture(Pattern::Text, 1920, 1080, algorithm);
        let mut driver = Driver::new(&fx);

        group.throughput(Throughput::Elements(fx.pixels()));
        group.bench_function(format!("{algorithm:?}"), |b| b.iter(|| driver.decode(&fx.frame)));
    }

    group.finish();
}

/// Per-stage cost on one tile's worth of real coefficients. Each stage runs on a
/// buffer restored to the state its predecessor would have left, so the inputs
/// are the ones the stage sees in production (coefficient magnitudes drive both
/// the RLGR and the DWT cost).
fn stage(c: &mut Criterion) {
    let mut group = c.benchmark_group("stage");
    // One tile of one component. Three components per tile, so multiply by 3 to
    // compare against the per-tile numbers.
    group.throughput(Throughput::Elements(
        u64::try_from(TILE * TILE).expect("tile area fits in u64"),
    ));

    for pattern in [Pattern::Desktop, Pattern::Text, Pattern::Noise] {
        let sf = stage_fixture(pattern, ALGO);
        let name = pattern.name();
        let mut out = vec![0i16; TILE * TILE];
        let mut temp = vec![0i16; TILE * TILE];

        group.bench_function(BenchmarkId::new("rlgr", name), |b| {
            b.iter(|| rlgr::decode(ALGO, &sf.encoded[0], &mut out).expect("rlgr"))
        });

        // Capture each stage's input once, then restore it per iteration. The
        // restore is a 8 KiB memcpy and is subtracted by measuring it too
        // (`stage/restore`), which is far cheaper than the stages themselves.
        let after_rlgr = {
            let mut v = vec![0i16; TILE * TILE];
            rlgr::decode(ALGO, &sf.encoded[0], &mut v).expect("rlgr");
            v
        };
        group.bench_function(BenchmarkId::new("subband", name), |b| {
            b.iter(|| {
                out.copy_from_slice(&after_rlgr);
                subband_reconstruction::decode(&mut out[4032..]);
            })
        });

        let after_subband = {
            let mut v = after_rlgr.clone();
            subband_reconstruction::decode(&mut v[4032..]);
            v
        };
        group.bench_function(BenchmarkId::new("quantization", name), |b| {
            b.iter(|| {
                out.copy_from_slice(&after_subband);
                quantization::decode(&mut out, &sf.quant);
            })
        });

        let after_quant = {
            let mut v = after_subband.clone();
            quantization::decode(&mut v, &sf.quant);
            v
        };
        group.bench_function(BenchmarkId::new("dwt", name), |b| {
            b.iter(|| {
                out.copy_from_slice(&after_quant);
                dwt::decode(&mut out, &mut temp);
            })
        });

        group.bench_function(BenchmarkId::new("restore", name), |b| {
            b.iter(|| out.copy_from_slice(&after_quant))
        });

        let planes = sf.planes();
        let mut rgba = vec![0u8; TILE * TILE * 4];
        group.bench_function(BenchmarkId::new("ycbcr_to_rgba", name), |b| {
            b.iter(|| {
                ycbcr_to_rgba(
                    YCbCrBuffer {
                        y: &planes[0],
                        cb: &planes[1],
                        cr: &planes[2],
                    },
                    &mut rgba,
                )
                .expect("ycbcr")
            })
        });
    }

    group.finish();
}

criterion_group!(benches, frame, tile, entropy, stage);
criterion_main!(benches);
