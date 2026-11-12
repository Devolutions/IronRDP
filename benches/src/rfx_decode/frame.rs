use criterion::{BenchmarkId, Criterion, Throughput};

use crate::ENTROPY_ALGORITHM;
use rfx_decode_fixtures::{Driver, Pattern, fixture};

/// Full-screen decode at common screen resolutions.
pub(crate) fn frame(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame");

    for (w, h) in [(1280u16, 720u16), (1920, 1080), (2560, 1440)] {
        for pattern in Pattern::ALL {
            let fx = fixture(pattern, w, h, ENTROPY_ALGORITHM);
            let mut driver = Driver::new(&fx);

            group.throughput(Throughput::Elements(fx.pixels()));
            group.bench_function(BenchmarkId::new(format!("{w}x{h}"), pattern.name()), |b| {
                b.iter(|| driver.decode(&fx.frame))
            });
        }
    }

    group.finish();
}
