use std::collections::HashMap;

use tinyjson::JsonValue;

use crate::prelude::*;
// NOTE: cargo-fuzz (libFuzzer) does not support Windows yet (coming soon?)

/// Enumerate fuzz targets by scanning `fuzz/fuzz_targets/*.rs`.
///
/// The fuzz targets directory is the single source of truth: each `.rs` file
/// there is a libFuzzer binary registered in `fuzz/Cargo.toml`. Discovering
/// them dynamically means the CI matrix picks up new targets automatically.
pub fn discover_targets() -> anyhow::Result<Vec<String>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .context("retrieve project root path")?
        .join("fuzz")
        .join("fuzz_targets");

    let mut targets: Vec<String> = std::fs::read_dir(&dir)
        .with_context(|| format!("read fuzz targets directory: {}", dir.display()))?
        .map(|entry| {
            let entry = entry.with_context(|| format!("read entry in {}", dir.display()))?;
            Ok(entry.path())
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .filter_map(|path| {
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                return None;
            }
            path.file_stem().and_then(|stem| stem.to_str()).map(str::to_owned)
        })
        .collect();

    targets.sort();
    Ok(targets)
}

pub fn corpus_minify(sh: &Shell, target: Option<String>) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-CORPUS-MINIFY");
    windows_skip!();

    let _guard = sh.push_dir("./fuzz");

    let targets = match target {
        Some(value) => vec![value],
        None => discover_targets()?,
    };

    for target in &targets {
        cmd!(sh, "rustup run {NIGHTLY_TOOLCHAIN} cargo fuzz cmin {target}").run()?;
    }

    Ok(())
}

pub fn corpus_fetch(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-CORPUS-FETCH");
    windows_skip!();

    cmd!(
        sh,
        "az storage blob download-batch --account-name fuzzingcorpus --source ironrdp --destination fuzz --output none"
    )
    .run()?;

    Ok(())
}

pub fn corpus_push(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-CORPUS-PUSH");
    windows_skip!();

    cmd!(
        sh,
        "az storage blob sync --account-name fuzzingcorpus --container ironrdp --source fuzz/corpus --destination corpus --delete-destination true --output none"
    )
    .run()?;

    cmd!(
        sh,
        "az storage blob sync --account-name fuzzingcorpus --container ironrdp --source fuzz/artifacts --destination artifacts --delete-destination true --output none"
    )
    .run()?;

    Ok(())
}

pub fn install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-INSTALL");
    windows_skip!();

    cargo_install(sh, &CARGO_FUZZ)?;

    cmd!(sh, "rustup install {NIGHTLY_TOOLCHAIN} --profile=minimal").run()?;

    // Required by `coverage` to merge raw profile data and render a report.
    // Without it, `cargo fuzz coverage` fails partway through with a missing
    // llvm-profdata error instead of at install time.
    cmd!(
        sh,
        "rustup component add llvm-tools-preview --toolchain {NIGHTLY_TOOLCHAIN}"
    )
    .run()?;

    Ok(())
}

pub fn run(sh: &Shell, duration: Option<u32>, target: Option<String>) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-RUN");
    windows_skip!();

    let _guard = sh.push_dir("./fuzz");

    let duration = duration.unwrap_or(5).to_string();
    let targets = match target {
        Some(value) => vec![value],
        None => discover_targets()?,
    };

    for target in &targets {
        cmd!(
            sh,
            "rustup run {NIGHTLY_TOOLCHAIN} cargo fuzz run {target} -- -max_total_time={duration} -timeout=10"
        )
        .run()?;
    }

    println!("All good!");

    Ok(())
}

/// Locate the pinned nightly toolchain's bundled LLVM tools directory
/// (`llvm-cov`, `llvm-profdata`), installed via the `llvm-tools-preview`
/// rustup component. Neither `rustup run` nor `PATH` exposes these: `cargo
/// fuzz coverage` itself locates `llvm-profdata` the same way internally,
/// by resolving the toolchain's own `rustlib` tree rather than relying on
/// whatever `llvm-cov` a plain `PATH` lookup might find (which can be an
/// unrelated system-wide LLVM install with an incompatible profile format).
fn llvm_tools_bin_dir(sh: &Shell) -> anyhow::Result<std::path::PathBuf> {
    let libdir = cmd!(sh, "rustc +{NIGHTLY_TOOLCHAIN} --print target-libdir")
        .read()
        .context("locate the pinned nightly toolchain's target-libdir")?;

    let bin_dir = std::path::Path::new(libdir.trim())
        .parent()
        .context("target-libdir has no parent directory")?
        .join("bin");

    anyhow::ensure!(
        bin_dir.join("llvm-cov").is_file(),
        "llvm-cov not found at {}; run `cargo xtask fuzz install` first",
        bin_dir.display()
    );

    Ok(bin_dir)
}

/// Run each target against its existing corpus (`cargo xtask fuzz corpus-fetch`
/// first, for a corpus worth reporting on) and print per-file line coverage.
///
/// Coverage-guided fuzzing typically plateaus around 12 hours in (Liyanage
/// et al., ICSE 2023); several targets here have run far longer than that
/// without any coverage-feedback check. Running a target for hours after
/// its coverage has stopped growing hides bug-finding opportunity
/// elsewhere; this surfaces that instead of leaving it implicit.
///
/// Only lines with nonzero coverage are printed, since the full report
/// otherwise lists every source file linked into the fuzz binary, most of
/// which the target never reaches and are not useful to see.
pub fn coverage(sh: &Shell, target: Option<String>) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-COVERAGE");
    windows_skip!();

    let _guard = sh.push_dir("./fuzz");

    let targets = match target {
        Some(value) => vec![value],
        None => discover_targets()?,
    };

    let llvm_cov = llvm_tools_bin_dir(sh)?.join("llvm-cov");

    for target in &targets {
        cmd!(sh, "rustup run {NIGHTLY_TOOLCHAIN} cargo fuzz coverage {target}").run()?;

        let binary = std::path::Path::new("target/x86_64-unknown-linux-gnu/coverage/x86_64-unknown-linux-gnu/release")
            .join(target);
        let profdata = format!("coverage/{target}/coverage.profdata");

        let report = cmd!(
            sh,
            "{llvm_cov} report {binary} --instr-profile={profdata} --ignore-filename-regex=cargo/registry|/rustc/|\\.cargo/"
        )
        .read()
        .with_context(|| format!("generate coverage report for {target}"))?;

        println!("--- {target} ---");
        for line in report.lines() {
            // A fully-untouched file's row carries exactly three "0.00%" columns
            // (regions, functions, lines; branches shows "-", not a percentage).
            // Any row with fewer is touched and worth keeping; the header, the
            // separator, and the TOTAL summary line never contain "0.00%" at
            // all and are always kept.
            if line.matches("0.00%").count() < 3 {
                println!("{line}");
            }
        }
    }

    Ok(())
}

/// Print each fuzz target, one per line. Useful for local discovery.
pub fn list_human() -> anyhow::Result<()> {
    for target in discover_targets()? {
        println!("{target}");
    }
    Ok(())
}

/// Emit a `matrix.include`-compatible JSON array on stdout, one entry per
/// discovered fuzz target. Suitable for piping into a GitHub Actions matrix:
///
/// ```yaml
/// - id: setup
///   run: echo "fuzz-matrix=$(cargo xtask fuzz list --format github-matrix)" >> "$GITHUB_OUTPUT"
/// ```
///
/// Each entry has the shape `{ "target": "<name>" }`.
pub fn list_github_matrix() -> anyhow::Result<()> {
    let items: Vec<JsonValue> = discover_targets()?
        .into_iter()
        .map(|name| {
            let mut obj = HashMap::new();
            obj.insert("target".to_owned(), JsonValue::String(name));
            JsonValue::Object(obj)
        })
        .collect();

    let json = JsonValue::Array(items);
    let stringified = json
        .stringify()
        .context("serialize fuzz matrix include array as JSON")?;
    println!("{stringified}");
    Ok(())
}
