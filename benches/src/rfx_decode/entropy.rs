use criterion::{Criterion, Throughput};
use ironrdp::pdu::codecs::rfx::EntropyAlgorithm;

use rfx_decode_fixtures::{Driver, Pattern, fixture};

/// Compares full-frame decoding with RLGR1 and RLGR3.
pub(crate) fn entropy(c: &mut Criterion) {
    let mut group = c.benchmark_group("entropy");

    for algorithm in [EntropyAlgorithm::Rlgr1, EntropyAlgorithm::Rlgr3] {
        let fx = fixture(Pattern::Text, 1920, 1080, algorithm);
        let mut driver = Driver::new(&fx);

        group.throughput(Throughput::Elements(fx.pixels()));
        group.bench_function(format!("{algorithm:?}"), |b| b.iter(|| driver.decode(&fx.frame)));
    }

    group.finish();
}
