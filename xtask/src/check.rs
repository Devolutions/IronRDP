use std::collections::BTreeSet;

use crate::prelude::*;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum ProtectedSetting {
    Toml {
        table: &'static str,
        key: &'static str,
        value: &'static str,
    },
}

struct ProtectedSettingContext {
    pathspec: &'static str,
    settings: &'static [ProtectedSetting],
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct ProtectedSettingOccurrence {
    path: String,
    setting: ProtectedSetting,
}

const PROTECTED_SETTING_CONTEXTS: &[ProtectedSettingContext] = &[ProtectedSettingContext {
    pathspec: ":(glob)**/Cargo.toml",
    settings: &[
        ProtectedSetting::Toml {
            table: "lib",
            key: "doctest",
            value: "false",
        },
        ProtectedSetting::Toml {
            table: "lib",
            key: "test",
            value: "false",
        },
    ],
}];

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
    let mut base_settings = BTreeSet::new();
    let mut head_settings = BTreeSet::new();

    for context in PROTECTED_SETTING_CONTEXTS {
        let pathspec = context.pathspec;
        let changes = cmd!(
            sh,
            "git diff --name-status --find-renames --diff-filter=MRT {base} {head} -- {pathspec}"
        )
        .read()
        .with_context(|| format!("compare files matching {pathspec}"))?;

        for change in changes.lines() {
            let mut fields = change.split('\t');
            let status = fields.next().context("missing protected-setting change status")?;
            let base_path = fields.next().context("missing protected-setting path")?;
            let head_path = if status.starts_with('R') {
                fields.next().context("missing renamed protected-setting path")?
            } else {
                base_path
            };

            let base_file = git_file(sh, base, base_path)?;
            let head_file = git_file(sh, head, head_path)?;
            base_settings.extend(protected_settings(&base_file, head_path, context.settings));
            head_settings.extend(protected_settings(&head_file, head_path, context.settings));
        }
    }

    let removals = base_settings.difference(&head_settings).collect::<Vec<_>>();

    if !removals.is_empty() {
        let removals = removals
            .into_iter()
            .map(|occurrence| {
                let ProtectedSetting::Toml { table, key, value } = occurrence.setting;
                format!("- {}: `[{table}] {key} = {value}`", occurrence.path)
            })
            .collect::<Vec<_>>()
            .join("\n");

        anyhow::bail!("protected settings were removed or changed:\n{removals}");
    }

    println!("All good!");

    Ok(())
}

fn git_file(sh: &Shell, revision: &str, path: &str) -> anyhow::Result<String> {
    let object = format!("{revision}:{path}");
    cmd!(sh, "git show {object}")
        .read()
        .with_context(|| format!("read {path} at {revision}"))
}

fn protected_settings(
    file: &str,
    path: &str,
    protected_settings: &[ProtectedSetting],
) -> BTreeSet<ProtectedSettingOccurrence> {
    let mut table = "";
    let mut occurrences = BTreeSet::new();

    for line in file.lines() {
        let line = line.split_once('#').map_or(line, |(line, _)| line).trim();

        if let Some(table_name) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']')) {
            table = table_name;
        } else if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            for setting in protected_settings {
                let ProtectedSetting::Toml {
                    table: protected_table,
                    key: protected_key,
                    value: protected_value,
                } = *setting;

                if (table, key, value) == (protected_table, protected_key, protected_value) {
                    occurrences.insert(ProtectedSettingOccurrence {
                        path: path.to_owned(),
                        setting: *setting,
                    });
                }
            }
        }
    }

    occurrences
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
    cmd!(sh, "{CARGO} test -p xtask --bin xtask --locked --no-run").run()?;
    cmd!(
        sh,
        "{CARGO} test -p ironrdp-testsuite-extra --test integration_tests_extra --no-default-features --features native-tls --locked --no-run"
    )
    .run()?;
    cmd!(
        sh,
        "{CARGO} test -p ironrdp-testsuite-extra --test integration_tests_extra --no-default-features --features native-tls,smartcard --locked --no-run"
    )
    .run()?;
    println!("All good!");
    Ok(())
}

pub fn tests_run(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TESTS-RUN");
    cmd!(sh, "{CARGO} test --workspace --locked").run()?;
    cmd!(sh, "{CARGO} test -p xtask --bin xtask --locked").run()?;
    cmd!(
        sh,
        "{CARGO} test -p ironrdp-testsuite-extra --test integration_tests_extra --no-default-features --features native-tls --locked"
    )
    .run()?;
    cmd!(
        sh,
        "{CARGO} test -p ironrdp-testsuite-extra --test integration_tests_extra --no-default-features --features native-tls,smartcard --locked"
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
