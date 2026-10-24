use core::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use ironrdp::graphics::color_conversion::{YCbCrBuffer, ycbcr_to_rgba};
use ironrdp::graphics::{dwt, quantization, rlgr, subband_reconstruction};

use crate::ENTROPY_ALGORITHM;
use rfx_decode_fixtures::{LL3_OFFSET, Pattern, TILE, stage_fixture};

/// Per-stage cost on one tile's worth of real coefficients.
/// Each stage starts from the output of its predecessor, restored outside the timed section.
pub(crate) fn stage(c: &mut Criterion) {
    let mut group = c.benchmark_group("stage");
    // Stages process one component, except color conversion, which processes all three.
    group.throughput(Throughput::Elements(
        u64::try_from(TILE * TILE).expect("tile area fits in u64"),
    ));

    for pattern in [Pattern::Desktop, Pattern::Text, Pattern::Noise] {
        let sf = stage_fixture(pattern, ENTROPY_ALGORITHM);
        let name = pattern.name();
        let mut out = vec![0i16; TILE * TILE];
        let mut temp = vec![0i16; TILE * TILE];

        group.bench_function(BenchmarkId::new("rlgr", name), |b| {
            b.iter(|| {
                rlgr::decode(ENTROPY_ALGORITHM, &sf.encoded[0], &mut out).expect("rlgr");
                black_box(&out);
            })
        });

        // Capture each stage's input once, then clone it in the untimed setup of
        // every batch so that only the stage itself is measured.
        let after_rlgr = {
            let mut v = vec![0i16; TILE * TILE];
            rlgr::decode(ENTROPY_ALGORITHM, &sf.encoded[0], &mut v).expect("rlgr");
            v
        };
        group.bench_function(BenchmarkId::new("subband", name), |b| {
            b.iter_batched_ref(
                || after_rlgr.clone(),
                |buf| {
                    subband_reconstruction::decode(&mut buf[LL3_OFFSET..]);
                    black_box(buf);
                },
                BatchSize::SmallInput,
            )
        });

        let after_subband = {
            let mut v = after_rlgr.clone();
            subband_reconstruction::decode(&mut v[LL3_OFFSET..]);
            v
        };
        group.bench_function(BenchmarkId::new("quantization", name), |b| {
            b.iter_batched_ref(
                || after_subband.clone(),
                |buf| {
                    quantization::decode(buf, &sf.quant);
                    black_box(buf);
                },
                BatchSize::SmallInput,
            )
        });

        let after_quant = {
            let mut v = after_subband.clone();
            quantization::decode(&mut v, &sf.quant);
            v
        };
        group.bench_function(BenchmarkId::new("dwt", name), |b| {
            b.iter_batched_ref(
                || after_quant.clone(),
                |buf| {
                    dwt::decode(buf, &mut temp);
                    black_box(buf);
                },
                BatchSize::SmallInput,
            )
        });

        let planes = sf.planes();
        let mut rgba = vec![0u8; TILE * TILE * 4 /* RGBA bytes per pixel */];
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
                .expect("ycbcr");
                black_box(&rgba);
            })
        });
    }

    group.finish();
}
