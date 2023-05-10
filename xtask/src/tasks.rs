use anyhow::Context as _;
use xshell::{cmd, Shell};

use crate::section::Section;

const CARGO: &str = env!("CARGO");
const CARGO_FUZZ_VERSION: &str = "0.11.2";
const LOCAL_CARGO_ROOT: &str = "./target/local_root/";
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

pub fn fuzz_run(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FUZZ-RUN");

    let _guard = sh.push_dir("./fuzz");

    for target in FUZZ_TARGETS {
        cmd!(
            sh,
            "../target/local_root/bin/cargo-fuzz run {target} -- -max_total_time=5s"
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
        cmd!(sh, "rustup run nightly cargo fuzz cmin {target}").run()?;
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

pub fn report_code_coverage(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("COVERAGE");

    println!("Remove leftovers");
    sh.remove_path("./fuzz/coverage/")?;
    sh.remove_path("./coverage/")?;

    sh.create_dir("./coverage/binaries")?;

    {
        // Fuzz coverage

        let _guard = sh.push_dir("./fuzz");

        cmd!(sh, "{CARGO} clean").run()?;

        for target in FUZZ_TARGETS {
            cmd!(sh, "rustup run nightly cargo fuzz coverage {target}").run()?;
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
        "grcov . ./fuzz
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
