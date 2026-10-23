#![allow(unused_crate_dependencies)] // Library and binary dependencies are not used by this target.

use core::hint::black_box;

#[cfg(feature = "codspeed")]
use codspeed_criterion_compat as criterion;
use criterion::{Criterion, criterion_group, criterion_main};
use ironrdp_bench::corpus::{CONNECTOR_BENCHMARK, Corpus, REPLAY_BENCHMARK};
use ironrdp_capture_replay::ReplayOptions;

fn replay(c: &mut Criterion) {
    let corpus = Corpus::load().expect("valid corpus manifest");
    let cache_dir = corpus.cache_dir();
    for capture in corpus.captures.iter().filter(|capture| capture.lists(REPLAY_BENCHMARK)) {
        // Criterion runs this closure only for selected benchmarks, so unselected captures are never loaded.
        c.bench_function(&format!("replay/{}", capture.id), |b| {
            let prepared = capture.prepare(&cache_dir).expect("cached capture must prepare");
            let summary = prepared
                .replay_with_options(ReplayOptions {
                    calculate_output_fingerprint: true,
                })
                .expect("capture must replay")
                .summary;
            capture
                .verify(&summary)
                .expect("capture replay must match its expectation");

            b.iter(|| black_box(prepared.replay_with_options(ReplayOptions::default())));
        });
    }
}

fn connector(c: &mut Criterion) {
    let corpus = Corpus::load().expect("valid corpus manifest");
    let cache_dir = corpus.cache_dir();
    for capture in corpus
        .captures
        .iter()
        .filter(|capture| capture.lists(CONNECTOR_BENCHMARK))
    {
        // Criterion runs this closure only for selected benchmarks, so unselected captures are never loaded.
        c.bench_function(&format!("connector/{}", capture.id), |b| {
            let connector = capture
                .prepare_connector(&cache_dir)
                .expect("cached capture must prepare");
            connector.verify().expect("connector run must match its expectation");

            b.iter(|| black_box(connector.replay()));
        });
    }
}

criterion_group!(benches, replay, connector);
criterion_main!(benches);
