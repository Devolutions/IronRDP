use criterion::{Criterion, Throughput};

use crate::ENTROPY_ALGORITHM;
use rfx_decode_fixtures::{Driver, Pattern, fixture};

/// One 64x64 tile, isolating per-tile cost from the framebuffer blit's cache behavior.
pub(crate) fn tile(c: &mut Criterion) {
    let mut group = c.benchmark_group("tile");

    for pattern in Pattern::ALL {
        let fx = fixture(pattern, 64, 64, ENTROPY_ALGORITHM);
        let mut driver = Driver::new(&fx);

        group.throughput(Throughput::Elements(fx.pixels()));
        group.bench_function(pattern.name(), |b| b.iter(|| driver.decode(&fx.frame)));
    }

    group.finish();
}
