#![allow(unused_crate_dependencies)] // Dependencies are used by the library target.
#![allow(clippy::print_stdout)]

use anyhow::Context as _;
use ironrdp_bench::corpus::{Corpus, REPLAY_BENCHMARK};
use ironrdp_capture_replay::ReplayOptions;

const USAGE: &str = "usage: corpus-bench fetch
       corpus-bench list
       corpus-bench check [--capture <ID>]
       corpus-bench replay --capture <ID> [--strict]";

fn main() -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["fetch"] => ironrdp_bench::corpus::fetch(&mut out),
        ["list"] => ironrdp_bench::corpus::list(&mut out),
        ["check"] => ironrdp_bench::corpus::check(None, &mut out),
        ["check", "--capture", id] => ironrdp_bench::corpus::check(Some(id), &mut out),
        ["replay", "--capture", id] => replay(id, false),
        ["replay", "--capture", id, "--strict"] => replay(id, true),
        _ => anyhow::bail!(USAGE),
    }
}

/// Replay one cached capture once and print a payload-free summary line.
///
/// Without `--strict`, the replay skips hashing framebuffer output, matching the timed replay.
fn replay(id: &str, full: bool) -> anyhow::Result<()> {
    let corpus = Corpus::load()?;
    let capture = corpus.benchmark_capture(REPLAY_BENCHMARK, id)?;
    let prepared = capture.prepare(&corpus.cache_dir())?;
    let summary = prepared
        .replay_with_options(ReplayOptions {
            calculate_output_fingerprint: full,
        })
        .with_context(|| format!("replay capture {id}"))?
        .summary;
    capture.verify(&summary)?;
    println!(
        "replay/{id}\tclient_pdus={}\tgraphics_updates={}\tgaps={}",
        summary.client_pdus, summary.graphics_updates, summary.gaps
    );
    Ok(())
}
