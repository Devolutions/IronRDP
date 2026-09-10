#![allow(unused_crate_dependencies)] // The package also contains standalone benchmark binaries.

use core::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use ironrdp_bench::connector_replay::{ConnectorReplayId, ConnectorReplayWorkload};
use ironrdp_bench::replay::{PartialReplayId, PartialReplayWorkload};

fn partial_replay(c: &mut Criterion) {
    for id in PartialReplayId::ALL {
        let name = format!("partial-replay/{}/processing", id.as_str());
        c.bench_function(&name, |b| {
            let workload = PartialReplayWorkload::prepare(id).expect("qualified partial replay workload must prepare");
            workload
                .verify()
                .expect("qualified partial replay workload must pass strict preflight");
            b.iter(|| {
                let measurement = workload
                    .replay()
                    .expect("qualified partial replay workload must execute");
                black_box(measurement)
            });
        });
    }
}

fn connector_replay(c: &mut Criterion) {
    for id in ConnectorReplayId::ALL {
        let name = format!("connector-replay/{}/connection-and-session", id.as_str());
        c.bench_function(&name, |b| {
            let workload =
                ConnectorReplayWorkload::prepare(id).expect("connector replay workload must prepare and preflight");
            workload
                .verify()
                .expect("connector replay workload must pass strict preflight");
            b.iter(|| {
                let measurement = workload.replay().expect("connector replay workload must execute");
                black_box(measurement)
            });
        });
    }
}

criterion_group!(benches, partial_replay, connector_replay);
criterion_main!(benches);
