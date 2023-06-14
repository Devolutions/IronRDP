use crate::prelude::*;

// NOTE: cargo-fuzz (libFuzzer) does not support Windows yet (coming soon?)

pub fn corpus_minify(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-CORPUS-MINIFY");
    windows_skip!();

    let _guard = sh.push_dir("./fuzz");

    for target in FUZZ_TARGETS {
        cmd!(sh, "rustup run nightly cargo fuzz cmin {target}")
            .env("RUSTUP_TOOLCHAIN", "nightly")
            .run()?;
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

    if !is_installed(sh, "cargo-fuzz") {
        // Install in debug because it's faster to compile and we don't need execution speed anyway.
        // cargo-fuzz version is pinned so we don’t get different versions without intervention.
        cmd!(
            sh,
            "{CARGO} install --debug --locked --root {LOCAL_CARGO_ROOT} cargo-fuzz@{CARGO_FUZZ_VERSION}"
        )
        .run()?;
    }

    cmd!(sh, "rustup install nightly --profile=minimal").run()?;

    Ok(())
}

pub fn run(sh: &Shell, duration: Option<u32>, target: Option<String>) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-RUN");
    windows_skip!();

    let _guard = sh.push_dir("./fuzz");

    let duration = duration.unwrap_or(5).to_string();
    let target_from_user = target.as_deref().map(|value| [value]);

    let targets = if let Some(targets) = &target_from_user {
        targets
    } else {
        FUZZ_TARGETS
    };

    for target in targets {
        cmd!(
            sh,
            "rustup run nightly cargo fuzz run {target} -- -max_total_time={duration}"
        )
        .env("RUSTUP_TOOLCHAIN", "nightly")
        .run()?;
    }

    println!("All good!");

    Ok(())
}
