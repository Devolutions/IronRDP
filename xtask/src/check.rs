use crate::prelude::*;

pub fn fmt(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FORMATTING");

    let output = cmd!(sh, "{CARGO} fmt --all -- --check").ignore_status().output()?;

    if !output.status.success() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        anyhow::bail!("Bad formatting, please run 'cargo +stable fmt --all'");
    }

    println!("All good!");

    Ok(())
}

pub fn lints(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("LINTS");

    // TODO: when 1.74 is released use `--keep-going`: https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#keep-going
    cmd!(
        sh,
        "{CARGO} clippy --workspace --all-targets --features helper,__bench --locked -- -D warnings"
    )
    .run()?;

    println!("All good!");

    Ok(())
}

pub fn typos(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TYPOS-CLI");

    if !is_installed(sh, "typos") {
        anyhow::bail!("`typos-cli` binary is missing. Please run `cargo xtask check install`.");
    }

    cmd!(sh, "typos").run()?;

    println!("All good!");
    Ok(())
}

pub fn dependencies(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("DEPENDENCIES");

    // Dependency-graph invariants that must hold to keep crate boundaries slim.
    // Each pair `(package, banned)` asserts that `package` has no transitive
    // (non-dev) edge to `banned`, ensuring consumers can depend on the
    // former without pulling in the latter’s graph.
    const FORBIDDEN: &[(&str, &str)] = &[("ironrdp-session", "ironrdp-connector"), ("ironrdp-session", "sspi")];

    let mut violations = Vec::new();

    for &(package, banned) in FORBIDDEN {
        // `cargo tree -i` inverts the graph to show what depends on `banned`,
        // scoped to `package`’s subtree. When there is no such edge, cargo exits
        // non-zero with a "did not match any packages" error; a successful,
        // non-empty output means the forbidden edge is present.
        let output = cmd!(sh, "{CARGO} tree -p {package} -e no-dev -i {banned}")
            .ignore_status()
            .quiet()
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let expected_no_match = format!("package ID specification `{banned}` did not match any packages");

        if output.status.success() && !stdout.trim().is_empty() {
            println!("Forbidden dependency edge: `{package}` depends on `{banned}`");
            print!("{stdout}");
            violations.push((package, banned));
        } else if output.status.success() || stderr.contains(expected_no_match.as_str()) {
            println!("`{package}` has no dependency on `{banned}` (good)");
        } else {
            print!("{stdout}");
            eprint!("{stderr}");
            anyhow::bail!("failed to inspect dependency edge `{package}` -> `{banned}`");
        }
    }

    if !violations.is_empty() {
        anyhow::bail!("forbidden dependency edge(s) detected, see output above");
    }

    println!("All good!");

    Ok(())
}

pub fn install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("CHECK-INSTALL");

    cargo_install(sh, &TYPOS_CLI)?;
    cargo_install(sh, &CARGO_HACK)?;

    Ok(())
}

pub fn tests_compile(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TESTS-COMPILE");
    cmd!(sh, "{CARGO} test --workspace --locked --no-run").run()?;
    println!("All good!");
    Ok(())
}

pub fn tests_run(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TESTS-RUN");
    cmd!(sh, "{CARGO} test --workspace --locked").run()?;
    println!("All good!");
    Ok(())
}

pub fn lock_files(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("CHECK-LOCKS");

    // Note that we can’t really use the --locked option of cargo, because to
    // run xtask, we need to compile it using cargo first, and thus the lock
    // files are already "refreshed" as far as cargo is concerned. Instead,
    // this task will check for modifications to the lock files using git-status
    // porcelain. The side benefit is that we can check for npm lock files too.

    const LOCK_FILES: &[&str] = &[
        "Cargo.lock",
        "fuzz/Cargo.lock",
        "web-client/iron-remote-desktop/package-lock.json",
        "web-client/iron-remote-desktop-rdp/package-lock.json",
        "web-client/iron-svelte-client/package-lock.json",
    ];

    let output = cmd!(sh, "git status --porcelain --untracked-files=no")
        .args(LOCK_FILES)
        .read()?;

    if !output.is_empty() {
        cmd!(sh, "git status").run()?;
        anyhow::bail!("one or more lock files are changed, you should commit those");
    }

    println!("All good!");

    Ok(())
}
