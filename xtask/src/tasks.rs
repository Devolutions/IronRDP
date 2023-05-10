use anyhow::Context as _;
use xshell::{cmd, Shell};

use crate::section::Section;

const CARGO: &str = env!("CARGO");
const CARGO_FUZZ_VERSION: &str = "0.11.2";
const GRCOV_VERSION: &str = "0.8.18";
const LOCAL_CARGO_ROOT: &str = ".cargo/local_root/";
const WASM_PACKAGES: &[&str] = &["ironrdp-web"];
const FUZZ_TARGETS: &[&str] = &["pdu_decoding", "rle_decompression", "bitmap_stream"];

pub fn check_formatting(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FORMATTING");

    let output = cmd!(sh, "{CARGO} fmt --all -- --check").ignore_status().output()?;

    if !output.status.success() {
        anyhow::bail!("Bad formatting, please run 'cargo +stable fmt --all'");
    }

    println!("All good!");

    Ok(())
}

pub fn run_tests(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TESTS");
    cmd!(sh, "{CARGO} test --workspace --locked").run()?;
    println!("All good!");
    Ok(())
}

pub fn check_lints(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("LINTS");
    cmd!(sh, "{CARGO} clippy --workspace --locked -- -D warnings").run()?;
    println!("All good!");
    Ok(())
}

pub fn check_wasm(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WASM-CHECK");

    for package in WASM_PACKAGES {
        println!("Check {package}");

        cmd!(
            sh,
            "{CARGO} rustc --locked --target wasm32-unknown-unknown --package {package} --lib --crate-type cdylib"
        )
        .run()?;

        // When building a library, `-` in the artifact name are replaced by `_`
        let artifact_name = format!("{}.wasm", package.replace('-', "_"));

        let output = cmd!(sh, "wasm2wat ./target/wasm32-unknown-unknown/debug/{artifact_name}").output()?;
        let stdout = std::str::from_utf8(&output.stdout).context("wasm2wat output is not valid UTF-8")?;

        if stdout.contains("import \"env\"") {
            anyhow::bail!("Found undefined symbols in generated wasm file");
        }
    }

    println!("All good!");

    Ok(())
}

pub fn fuzz_run(sh: &Shell, duration: Option<u32>, target: Option<String>) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-RUN");

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
            "../{LOCAL_CARGO_ROOT}/bin/cargo-fuzz run {target} -- -max_total_time={duration}"
        )
        .env("RUSTUP_TOOLCHAIN", "nightly")
        .run()?;
    }

    println!("All good!");

    Ok(())
}

pub fn fuzz_corpus_minify(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-CORPUS-MINIFY");

    let _guard = sh.push_dir("./fuzz");

    for target in FUZZ_TARGETS {
        cmd!(sh, "../{LOCAL_CARGO_ROOT}/bin/cargo-fuzz cmin {target}")
            .env("RUSTUP_TOOLCHAIN", "nightly")
            .run()?;
    }

    Ok(())
}

pub fn fuzz_corpus_fetch(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-CORPUS-FETCH");

    cmd!(
        sh,
        "az storage blob download-batch --account-name fuzzingcorpus --source ironrdp --destination fuzz --output none"
    )
    .run()?;

    Ok(())
}

pub fn fuzz_corpus_push(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-CORPUS-PUSH");

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

pub fn fuzz_install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-INSTALL");

    let cargo_fuzz_path: std::path::PathBuf = [LOCAL_CARGO_ROOT, "bin", "cargo-fuzz"].iter().collect();

    if !sh.path_exists(cargo_fuzz_path) {
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

pub fn svelte_run(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("SVELTE-RUN");

    {
        let _guard = sh.push_dir("./web-client/iron-remote-gui");
        cmd!(sh, "npm install").run()?;
    }

    {
        let _guard = sh.push_dir("./web-client/iron-svelte-client");
        cmd!(sh, "npm install").run()?;
        cmd!(sh, "npm run dev-all").run()?;
    }

    Ok(())
}

pub fn coverage_install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("COVERAGE-INSTALL");

    cmd!(sh, "rustup install nightly --profile=minimal").run()?;
    cmd!(sh, "rustup component add --toolchain nightly llvm-tools-preview").run()?;
    cmd!(sh, "rustup component add llvm-tools-preview").run()?;

    cmd!(
        sh,
        "{CARGO} install --debug --locked --root {LOCAL_CARGO_ROOT} cargo-fuzz@{CARGO_FUZZ_VERSION}"
    )
    .run()?;

    cmd!(
        sh,
        "{CARGO} install --debug --locked --root {LOCAL_CARGO_ROOT} grcov@{GRCOV_VERSION}"
    )
    .run()?;

    Ok(())
}

pub fn coverage_report(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("COVERAGE-REPORT");

    println!("Remove leftovers");
    sh.remove_path("./fuzz/coverage/")?;
    sh.remove_path("./coverage/")?;

    sh.create_dir("./coverage/binaries")?;

    {
        // Fuzz coverage

        let _guard = sh.push_dir("./fuzz");

        cmd!(sh, "{CARGO} clean").run()?;

        for target in FUZZ_TARGETS {
            cmd!(sh, "../{LOCAL_CARGO_ROOT}/bin/cargo-fuzz coverage {target}")
                .env("RUSTUP_TOOLCHAIN", "nightly")
                .run()?;
        }

        cmd!(sh, "cp -r ./target ../coverage/binaries/").run()?;
    }

    {
        // Test coverage

        cmd!(sh, "{CARGO} clean").run()?;

        cmd!(sh, "rustup run nightly cargo test --workspace")
            .env("CARGO_INCREMENTAL", "0")
            .env("RUSTFLAGS", "-C instrument-coverage")
            .env("LLVM_PROFILE_FILE", "./coverage/default-%m-%p.profraw")
            .run()?;

        cmd!(sh, "cp -r ./target/debug ./coverage/binaries/").run()?;
    }

    sh.create_dir("./docs")?;

    cmd!(
        sh,
        "./{LOCAL_CARGO_ROOT}/bin/grcov . ./fuzz
        --source-dir .
        --binary-path ./coverage/binaries/
        --output-type html
        --branch
        --ignore-not-existing
        --ignore xtask/*
        --ignore src/*
        --ignore **/tests/*
        --ignore crates/*-generators/*
        --ignore crates/web/*
        --ignore crates/client/*
        --ignore crates/glutin-renderer/*
        --ignore crates/glutin-client/*
        --ignore crates/replay-client/*
        --ignore crates/tls/*
        --ignore fuzz/fuzz_targets/*
        --ignore target/*
        --ignore fuzz/target/*
        --excl-start begin-no-coverage
        --excl-stop end-no-coverage
        -o ./docs/coverage"
    )
    .run()?;

    println!("Code coverage report available in `./docs/coverage` folder");

    println!("Clean up");

    sh.remove_path("./coverage/")?;
    sh.remove_path("./fuzz/coverage/")?;
    sh.remove_path("./xtask/coverage/")?;

    sh.read_dir("./crates")?
        .into_iter()
        .try_for_each(|crate_path| -> xshell::Result<()> {
            for path in sh.read_dir(crate_path)? {
                if path.ends_with("coverage") {
                    sh.remove_path(path)?;
                }
            }
            Ok(())
        })?;

    Ok(())
}

pub fn clean_workspace(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("CLEAN");
    cmd!(sh, "{CARGO} clean").run()?;
    Ok(())
}

pub fn wasm_install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WASM-INSTALL");

    cmd!(sh, "rustup target add wasm32-unknown-unknown").run()?;

    Ok(())
}
