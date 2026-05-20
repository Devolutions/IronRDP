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
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
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

/// Print each fuzz target, one per line. Useful for local discovery.
pub fn list_human() -> anyhow::Result<()> {
    for target in discover_targets()? {
        println!("{target}");
    }
    Ok(())
}

/// Emit a `matrix.include`-compatible JSON array on stdout. CI consumes this
/// via `fromJson(steps.setup.outputs.fuzz-matrix)` to fan out one job per
/// fuzz target without hardcoding the list in the workflow.
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
