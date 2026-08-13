use std::collections::BTreeMap;

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

pub fn test_settings(sh: &Shell, base: &str, head: &str) -> anyhow::Result<()> {
    let _s = Section::new("TEST-SETTINGS");
    let manifest_pathspec = ":(glob)**/Cargo.toml";
    let diff = cmd!(
        sh,
        "git diff --unified=0 --no-ext-diff --no-textconv --diff-filter=ACMRT {base} {head} -- {manifest_pathspec}"
    )
    .read()
    .context("compare Cargo manifests")?;
    let removals = protected_setting_removals(&diff);

    if !removals.is_empty() {
        let removals = removals
            .into_iter()
            .map(|(path, setting)| format!("- {path}: `{setting} = false`"))
            .collect::<Vec<_>>()
            .join("\n");

        anyhow::bail!(
            "protected Cargo test settings were removed:\n{removals}\n\
             keep library test harnesses disabled and place tests in the testsuite crates as described in ARCHITECTURE.md"
        );
    }

    println!("All good!");

    Ok(())
}

fn protected_setting_removals(diff: &str) -> Vec<(String, &'static str)> {
    let mut path = None;
    let mut changes = BTreeMap::<(String, &'static str), i32>::new();

    for line in diff.lines() {
        if let Some(new_path) = line.strip_prefix("+++ b/") {
            path = Some(new_path.to_owned());
        } else if let Some(line) = line.strip_prefix('-') {
            if let (Some(path), Some(setting)) = (&path, protected_setting(line)) {
                *changes.entry((path.clone(), setting)).or_default() -= 1;
            }
        } else if let Some(line) = line.strip_prefix('+')
            && let (Some(path), Some(setting)) = (&path, protected_setting(line))
        {
            *changes.entry((path.clone(), setting)).or_default() += 1;
        }
    }

    changes
        .into_iter()
        .filter_map(|(setting, count)| (count < 0).then_some(setting))
        .collect()
}

fn protected_setting(line: &str) -> Option<&'static str> {
    let line = line.split_once('#').map_or(line, |(line, _)| line).trim();
    let (key, value) = line.split_once('=')?;

    if value.trim() != "false" {
        return None;
    }

    match key.trim() {
        "doctest" => Some("doctest"),
        "test" => Some("test"),
        _ => None,
    }
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
    cmd!(
        sh,
        "{CARGO} test -p ironrdp-tls --test native_tls --features native-tls --locked --no-run"
    )
    .run()?;
    println!("All good!");
    Ok(())
}

pub fn tests_run(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TESTS-RUN");
    cmd!(sh, "{CARGO} test --workspace --locked").run()?;
    cmd!(
        sh,
        "{CARGO} test -p ironrdp-tls --test native_tls --features native-tls --locked"
    )
    .run()?;
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
