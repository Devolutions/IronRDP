use crate::prelude::*;

pub fn install(sh: &Shell) -> anyhow::Result<()> {
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

pub fn report(sh: &Shell) -> anyhow::Result<()> {
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
