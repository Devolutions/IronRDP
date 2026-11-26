use criterion::{BenchmarkId, Criterion, Throughput};
use ironrdp::graphics::progressive::ProgressiveDecoder;

use rfx_decode_fixtures::{Pattern, progressive_fixture};

const SURFACE_ID: u16 = 0;
const CODEC_CONTEXT_ID: u32 = 0;

/// Full-screen RemoteFX Progressive update through `ProgressiveDecoder::decode_bitmap`.
/// Build with the `rayon` feature to decode the region's tiles on rayon's pool.
pub(crate) fn progressive(c: &mut Criterion) {
    let mut group = c.benchmark_group("progressive");

    let (w, h) = (1920u16, 1080u16);
    for pattern in Pattern::ALL {
        let fx = progressive_fixture(pattern, w, h);

        // The decoder keeps the surface's tile grid and each tile's reference across iterations.
        // Priming with one frame allocates that state, so every timed decode repaints existing tiles, as a server repainting the screen does.
        let mut decoder = ProgressiveDecoder::new();
        decoder
            .decode_bitmap(SURFACE_ID, CODEC_CONTEXT_ID, w, h, &fx.init)
            .expect("decode context");
        decoder
            .decode_bitmap(SURFACE_ID, CODEC_CONTEXT_ID, w, h, &fx.frame)
            .expect("decode first frame");

        group.throughput(Throughput::Elements(fx.pixels()));
        group.bench_function(BenchmarkId::new(format!("{w}x{h}"), pattern.name()), |b| {
            b.iter(|| {
                decoder
                    .decode_bitmap(SURFACE_ID, CODEC_CONTEXT_ID, w, h, &fx.frame)
                    .expect("decode frame")
            })
        });
    }

    group.finish();
}
