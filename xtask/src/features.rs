use core::fmt::Write as _;
use std::collections::HashMap;
use std::io::Write as _;

use tinyjson::JsonValue;

use crate::prelude::*;

/// One feature-check case in the matrix. Each case is independently runnable via
/// `cargo xtask check features --case <name>`, which is the local reproducer for
/// any failure surfaced by the CI fan-out.
pub struct FeatureCheckCase {
    /// Case identifier of the form `scope/feature-or-strategy`.
    pub name: &'static str,
    pub invocation: Invocation,
}

/// What the case actually runs.
pub enum Invocation {
    /// Direct `cargo check` for a single package with explicit feature selection.
    /// Locks the curated invariants (notably the `arbitrary` discipline and the
    /// std/alloc/no_std cascade paths) that the powerset alone does not pin down.
    CargoCheck {
        package: &'static str,
        no_default_features: bool,
        features: &'static [&'static str],
    },
    /// `cargo hack check --feature-powerset` over a package group with a depth cap.
    /// The workspace-level `EXCLUDE_FEATURES` list applies to every powerset case
    /// so internal-only flags do not skew the matrix. `extra_args` is appended
    /// verbatim, for cases that need to skip `--no-default-features` (TLS) or
    /// otherwise diverge from the default invocation.
    CargoHack {
        packages: &'static [&'static str],
        depth: u8,
        extra_args: &'static [&'static str],
    },
}

/// Features that the powerset should never enumerate.
///   `__bench` and `__test` are private features that pull `visibility` for the
///   testsuite; the existing lints job exercises them via `--features helper,__bench`.
const EXCLUDE_FEATURES: &[&str] = &["__bench", "__test"];

/// The canonical case set. Adding a new feature-matrix check means adding an entry
/// here; the CI matrix discovers it automatically through `--list`.
pub fn cases() -> &'static [FeatureCheckCase] {
    CASES
}

const CASES: &[FeatureCheckCase] = &[
    // Per-crate curated invariants.
    FeatureCheckCase {
        name: "ironrdp-core/alloc",
        invocation: Invocation::CargoCheck {
            package: "ironrdp-core",
            no_default_features: true,
            features: &["alloc"],
        },
    },
    FeatureCheckCase {
        name: "ironrdp-core/std",
        invocation: Invocation::CargoCheck {
            package: "ironrdp-core",
            no_default_features: false,
            features: &["std"],
        },
    },
    FeatureCheckCase {
        name: "ironrdp-pdu/std",
        invocation: Invocation::CargoCheck {
            package: "ironrdp-pdu",
            no_default_features: false,
            features: &["std"],
        },
    },
    FeatureCheckCase {
        name: "ironrdp-pdu/arbitrary",
        invocation: Invocation::CargoCheck {
            package: "ironrdp-pdu",
            no_default_features: false,
            features: &["arbitrary"],
        },
    },
    FeatureCheckCase {
        name: "ironrdp-pdu/arbitrary-alloc",
        invocation: Invocation::CargoCheck {
            package: "ironrdp-pdu",
            no_default_features: true,
            features: &["arbitrary", "alloc"],
        },
    },
    FeatureCheckCase {
        name: "ironrdp-egfx/arbitrary",
        invocation: Invocation::CargoCheck {
            package: "ironrdp-egfx",
            no_default_features: false,
            features: &["arbitrary"],
        },
    },
    // Workspace powerset, partitioned by layer so each fan-out worker stays bounded.
    // Adding a new crate to a group means the powerset picks it up on the next run.
    FeatureCheckCase {
        name: "workspace/powerset-foundation",
        invocation: Invocation::CargoHack {
            packages: &["ironrdp-core", "ironrdp-error", "ironrdp-str", "ironrdp-bulk"],
            depth: 2,
            extra_args: &[],
        },
    },
    FeatureCheckCase {
        name: "workspace/powerset-pdu",
        invocation: Invocation::CargoHack {
            packages: &["ironrdp-pdu", "ironrdp-graphics"],
            depth: 2,
            extra_args: &[],
        },
    },
    FeatureCheckCase {
        name: "workspace/powerset-channels",
        invocation: Invocation::CargoHack {
            packages: &[
                "ironrdp-cliprdr",
                "ironrdp-rdpdr",
                "ironrdp-rdpsnd",
                "ironrdp-egfx",
                "ironrdp-dvc",
                "ironrdp-svc",
                "ironrdp-input",
                "ironrdp-ainput",
                "ironrdp-displaycontrol",
                "ironrdp-rdpeusb",
            ],
            depth: 2,
            extra_args: &[],
        },
    },
    FeatureCheckCase {
        name: "workspace/powerset-connector-session",
        invocation: Invocation::CargoHack {
            packages: &["ironrdp-connector", "ironrdp-session", "ironrdp-acceptor"],
            depth: 2,
            extra_args: &[],
        },
    },
    FeatureCheckCase {
        name: "workspace/powerset-runtime",
        invocation: Invocation::CargoHack {
            packages: &["ironrdp-async", "ironrdp-blocking", "ironrdp-tokio", "ironrdp-server"],
            depth: 2,
            extra_args: &[],
        },
    },
    // `ironrdp-tls`, `ironrdp-client`, `ironrdp-mstsgu` are intentionally
    // outside this initial case set. The `exactly-one-of` TLS-backend
    // constraint on `ironrdp-tls` needs `--mutually-exclusive-features`,
    // `--at-least-one-of`, and `--exclude-no-default-features` on the
    // cargo-hack invocation (cargo-hack does not honor
    // `package.metadata.cargo-hack`), and the powerset surfaces a latent
    // bug in `extract_tls_server_public_key` that uses `x509_cert::*`
    // unconditionally instead of gating on `rustls | native-tls`. Both
    // are tractable but out of scope for this gate's initial landing.
    // The regular `Checks` job already exercises all three crates with
    // their default features.
];

/// Run every case sequentially. Mirrors what a contributor gets locally with
/// `cargo xtask check features`; CI uses the per-case mode instead.
pub fn run_all(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FEATURE-MATRIX");

    if !is_installed(sh, &CARGO_HACK) {
        anyhow::bail!("`cargo-hack` is missing. Please run `cargo xtask check install`.");
    }

    for case in cases() {
        run_one(sh, case)?;
    }

    println!("All good!");
    Ok(())
}

/// Run a single case. Equivalent to `cargo xtask check features --case <name>`.
pub fn run_case(sh: &Shell, case_name: &str) -> anyhow::Result<()> {
    let case = cases().iter().find(|c| c.name == case_name).with_context(|| {
        format!("unknown case: `{case_name}`; run `cargo xtask check features --list` to enumerate")
    })?;

    let result = run_one(sh, case);
    let status = if result.is_ok() { "pass" } else { "fail" };
    let _ = write_step_summary(case, status);
    result
}

fn run_one(sh: &Shell, case: &FeatureCheckCase) -> anyhow::Result<()> {
    let _s = Section::new(case.name);

    match &case.invocation {
        Invocation::CargoCheck {
            package,
            no_default_features,
            features,
        } => {
            let mut args: Vec<String> = vec!["check".into(), "-p".into(), (*package).into(), "--locked".into()];
            if *no_default_features {
                args.push("--no-default-features".into());
            }
            if !features.is_empty() {
                args.push("--features".into());
                args.push(features.join(","));
            }
            cmd!(sh, "{CARGO}").args(&args).run()?;
        }
        Invocation::CargoHack {
            packages,
            depth,
            extra_args,
        } => {
            if !is_installed(sh, &CARGO_HACK) {
                anyhow::bail!("`cargo-hack` is missing. Please run `cargo xtask check install`.");
            }

            let depth_str = depth.to_string();
            let exclude_features = EXCLUDE_FEATURES.join(",");
            // `--no-dev-deps` mutates `Cargo.toml` while cargo-hack runs,
            // which would force Cargo.lock to drift and conflict with
            // `--locked`. Keeping dev-deps in the resolution costs a small
            // amount of extra type-checking but preserves lockfile
            // reproducibility for the powerset.
            let mut args: Vec<String> = vec![
                "hack".into(),
                "check".into(),
                "--locked".into(),
                "--feature-powerset".into(),
                "--depth".into(),
                depth_str,
                "--exclude-features".into(),
                exclude_features,
            ];
            for pkg in *packages {
                args.push("-p".into());
                args.push((*pkg).into());
            }
            for extra in *extra_args {
                args.push((*extra).into());
            }
            cmd!(sh, "{CARGO}").args(&args).run()?;
        }
    }

    Ok(())
}

/// Print each case, one per line. Useful for local discovery.
pub fn list_human() -> anyhow::Result<()> {
    for case in cases() {
        println!("{}", case.name);
    }
    Ok(())
}

/// Emit a `matrix.include`-compatible JSON array on stdout. CI consumes this via
/// `fromJson(steps.setup.outputs.feature-matrix)`.
pub fn list_github_matrix() -> anyhow::Result<()> {
    let items: Vec<JsonValue> = cases()
        .iter()
        .map(|c| {
            let mut obj = HashMap::new();
            obj.insert("case".to_owned(), JsonValue::String(c.name.to_owned()));
            JsonValue::Object(obj)
        })
        .collect();

    let json = JsonValue::Array(items);
    let stringified = json
        .stringify()
        .context("serialize feature-matrix include array as JSON")?;
    println!("{stringified}");
    Ok(())
}

/// Append a brief markdown line to `$GITHUB_STEP_SUMMARY` so the matrix fan-out
/// surfaces in the workflow summary view without expanding logs. No-op locally.
fn write_step_summary(case: &FeatureCheckCase, status: &str) -> anyhow::Result<()> {
    let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") else {
        return Ok(());
    };

    let mut line = String::new();
    writeln!(line, "- `{}`: **{}**", case.name, status)?;

    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(line.as_bytes())?;

    Ok(())
}
