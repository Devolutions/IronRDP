use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;
use sha2::{Digest as _, Sha256};
use xshell::{Shell, cmd};

use crate::project_root;

const MANIFEST_PATH: &str = "crates/ironrdp-bench/corpus.toml";
const CACHE_ROOT: &str = "bench-data/wireshark-rdp";
const UPSTREAM_REPOSITORY: &str = "awakecoding/wireshark-rdp";

#[derive(Debug, Eq, PartialEq)]
struct Corpus {
    /// Immutable upstream revision that identifies the cached capture set.
    revision: String,
    /// Captures and strict replay expectations.
    captures: Vec<Capture>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Capture {
    /// Stable manifest identifier.
    id: String,
    /// Pinned upstream file name.
    file: String,
    /// Expected SHA-256 digest.
    sha256: String,
    /// Recorded upstream scenario description.
    intent: String,
    /// Optional focused performance workload for this capture.
    performance: Option<PerformanceWorkload>,
    /// Observed replay behavior for this exact capture revision.
    expect: ReplayExpectation,
}

/// A focused performance workload declared by the corpus manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PerformanceWorkload {
    /// Exact Criterion benchmark identifier.
    criterion: String,
}

/// Strict expected result for one capture replay.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayExpectation {
    /// Declared outcome category.
    outcome: ReplayOutcome,
    /// Failing replay stage for unsupported outcomes.
    stage: Option<String>,
    /// Stable failing replay reason for unsupported outcomes.
    reason: Option<String>,
    /// Exact counters and output identity for completed and partial outcomes.
    summary: Option<ReplaySummaryExpectation>,
}

/// Stable category for a qualified capture replay.
///
/// Add a rejected outcome only when the replay decodes protocol rejection evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplayOutcome {
    /// Replay reached a clean completed state.
    Complete,
    /// Replay produced useful output but recorded known gaps.
    Partial,
    /// The capture requires a replay capability that is not implemented.
    Unsupported,
}

/// Expected payload-free counters and output identity for a successful replay.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplaySummaryExpectation {
    /// Exact values keyed by the stable summary field names.
    values: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Eq, PartialEq)]
enum CacheState {
    Missing,
    Valid,
    Corrupt(String),
}

pub fn corpus_fetch(sh: &Shell) -> anyhow::Result<()> {
    let corpus = load_corpus()?;
    let cache_dir = project_root().join(CACHE_ROOT).join(&corpus.revision).join("captures");

    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("create corpus cache directory: {}", cache_dir.display()))?;

    for capture in &corpus.captures {
        let cache_path = cache_dir.join(&capture.file);

        match inspect_cache(&cache_path, capture)? {
            CacheState::Valid => {
                println!("Using verified cache: {}", capture.file);
                continue;
            }
            CacheState::Corrupt(reason) => {
                println!("Repairing corrupt cache entry {}: {reason}", capture.file);
            }
            CacheState::Missing => {
                println!("Fetching: {}", capture.file);
            }
        }

        let url = format!(
            "https://raw.githubusercontent.com/{UPSTREAM_REPOSITORY}/{}/captures/{}",
            corpus.revision, capture.file
        );
        let temporary_path = temporary_path(&cache_path)?;

        download(sh, &url, &temporary_path, &capture.sha256)?;

        install_capture(&temporary_path, &cache_path)?;
        println!("Fetched and verified: {}", capture.file);
    }
    Ok(())
}

pub fn corpus_list() -> anyhow::Result<()> {
    print!("{}", format_list(&load_corpus()?));
    Ok(())
}

/// Replay verified cached captures and enforce their qualified expectations.
pub fn corpus_replay(selector: Option<&str>) -> anyhow::Result<()> {
    let corpus = load_corpus()?;
    let cache_dir = project_root().join(CACHE_ROOT).join(&corpus.revision).join("captures");
    let captures = match selector {
        Some(selector) => corpus
            .captures
            .iter()
            .filter(|capture| capture.id == selector)
            .collect::<Vec<_>>(),
        None => corpus.captures.iter().collect(),
    };
    anyhow::ensure!(
        !captures.is_empty(),
        "unknown benchmark capture selector: {}",
        selector.unwrap_or_default()
    );

    let mut complete = 0;
    let mut partial = 0;
    let mut unsupported = 0;
    for capture in captures {
        let capture_path = cache_dir.join(&capture.file);
        verify_file(&capture_path, &capture.sha256)
            .with_context(|| format!("verify cached benchmark capture: {}", capture.file))?;
        let observed = run_headless_replay(&capture_path)?;
        validate_replay_expectation(capture, &observed)?;
        let outcome = observed.outcome()?;
        match outcome {
            ReplayOutcome::Complete => complete += 1,
            ReplayOutcome::Partial => partial += 1,
            ReplayOutcome::Unsupported => unsupported += 1,
        }
        println!("{}: {}", capture.id, outcome.name());
    }

    println!(
        "Qualified replay outcomes: complete={}, partial={}, unsupported={}",
        complete, partial, unsupported,
    );
    Ok(())
}

/// Run the manifest-declared Criterion workload for one eligible capture.
pub fn capture_replay_benchmark(selector: &str) -> anyhow::Result<()> {
    let corpus = load_corpus()?;
    let criterion = capture_replay_criterion(&corpus, selector)?;
    let status = Command::new(env!("CARGO"))
        .current_dir(project_root())
        .args([
            "bench",
            "-p",
            "ironrdp-bench",
            "--bench",
            "capture_replay",
            "--locked",
            "--",
        ])
        .arg(criterion)
        .arg("--exact")
        .status()
        .context("run selected capture replay benchmark")?;
    anyhow::ensure!(status.success(), "selected capture replay benchmark failed: {selector}");
    Ok(())
}

fn capture_replay_criterion<'a>(corpus: &'a Corpus, selector: &str) -> anyhow::Result<&'a str> {
    let capture = corpus
        .captures
        .iter()
        .find(|capture| capture.id == selector)
        .with_context(|| format!("unknown benchmark capture selector: {selector}"))?;
    capture
        .performance
        .as_ref()
        .map(|performance| performance.criterion.as_str())
        .with_context(|| format!("capture is not eligible for a performance benchmark: {selector}"))
}

#[derive(Debug, Eq, PartialEq)]
enum ObservedReplay {
    Success(ReplaySummaryExpectation),
    Failure { stage: String, reason: String },
}

impl ObservedReplay {
    fn outcome(&self) -> anyhow::Result<ReplayOutcome> {
        match self {
            Self::Success(summary) if summary_has_gaps(summary) || summary_lifecycle(summary) != Some("active") => {
                Ok(ReplayOutcome::Partial)
            }
            Self::Success(_) => Ok(ReplayOutcome::Complete),
            Self::Failure { reason, .. } if is_unsupported_reason(reason) => Ok(ReplayOutcome::Unsupported),
            Self::Failure { stage, reason } => anyhow::bail!(
                "unqualified headless replay failure: {stage}:{reason}; add explicit support or classify it as unsupported"
            ),
        }
    }
}

impl ReplayOutcome {
    const fn name(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Unsupported => "unsupported",
        }
    }
}

fn run_headless_replay(capture: &Path) -> anyhow::Result<ObservedReplay> {
    let output = Command::new(env!("CARGO"))
        .current_dir(project_root())
        .args([
            "run",
            "--quiet",
            "--locked",
            "-p",
            "ironrdp-capture-replay",
            "--bin",
            "ironrdp-capture-replay",
            "--",
        ])
        .arg("--summary")
        .arg(capture)
        .output()
        .with_context(|| format!("run headless replay: {}", capture.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "headless replay command failed for {}: {}",
        capture.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    parse_headless_summary(&String::from_utf8(output.stdout).context("headless replay output is not UTF-8")?)
}

fn parse_headless_summary(output: &str) -> anyhow::Result<ObservedReplay> {
    let line = output.lines().filter(|line| !line.is_empty()).collect::<Vec<_>>();
    anyhow::ensure!(line.len() == 1, "headless replay must emit exactly one summary line");
    let mut fields = std::collections::BTreeMap::new();
    for field in line[0].split('\t') {
        let (key, value) = field.split_once('=').context("invalid headless replay summary field")?;
        anyhow::ensure!(
            fields.insert(key, value).is_none(),
            "duplicate headless replay summary field: {key}"
        );
    }
    match fields.remove("status") {
        Some("ok") => {
            anyhow::ensure!(
                fields.len() == SUMMARY_KEYS.len(),
                "headless replay summary has unexpected fields"
            );
            let mut values = std::collections::BTreeMap::new();
            for key in SUMMARY_KEYS {
                let output_key = key.replace('_', "-");
                let value = fields
                    .remove(output_key.as_str())
                    .context("headless replay summary is missing a field")?;
                values.insert((*key).to_owned(), value.to_owned());
            }
            Ok(ObservedReplay::Success(ReplaySummaryExpectation { values }))
        }
        Some("error") => {
            let stage = fields.remove("stage").context("headless replay failure has no stage")?;
            let reason = fields
                .remove("reason")
                .context("headless replay failure has no reason")?;
            anyhow::ensure!(fields.is_empty(), "headless replay failure has unexpected fields");
            Ok(ObservedReplay::Failure {
                stage: stage.to_owned(),
                reason: reason.to_owned(),
            })
        }
        Some(status) => anyhow::bail!("unknown headless replay status: {status}"),
        None => anyhow::bail!("headless replay summary has no status"),
    }
}

fn validate_replay_expectation(capture: &Capture, observed: &ObservedReplay) -> anyhow::Result<()> {
    anyhow::ensure!(
        capture.expect.outcome == observed.outcome()?,
        "replay outcome mismatch for {}: expected {}, got {}",
        capture.id,
        capture.expect.outcome.name(),
        observed.outcome()?.name()
    );
    match (capture.expect.summary.as_ref(), observed) {
        (Some(expected), ObservedReplay::Success(actual)) => anyhow::ensure!(
            expected == actual,
            "replay summary mismatch for {}: update the qualified manifest explicitly",
            capture.id
        ),
        (None, ObservedReplay::Failure { stage, reason }) => anyhow::ensure!(
            capture.expect.stage.as_deref() == Some(stage) && capture.expect.reason.as_deref() == Some(reason),
            "replay failure mismatch for {}: expected {}:{}, got {stage}:{reason}",
            capture.id,
            capture.expect.stage.as_deref().unwrap_or_default(),
            capture.expect.reason.as_deref().unwrap_or_default(),
        ),
        _ => anyhow::bail!("replay expectation shape mismatch for {}", capture.id),
    }
    Ok(())
}

fn summary_has_gaps(summary: &ReplaySummaryExpectation) -> bool {
    SUMMARY_KEYS
        .iter()
        .filter(|key| key.ends_with("_gaps"))
        .any(|key| summary.values.get(*key).is_some_and(|value| value != "0"))
}

fn summary_lifecycle(summary: &ReplaySummaryExpectation) -> Option<&str> {
    summary.values.get("lifecycle").map(String::as_str)
}

fn is_unsupported_reason(reason: &str) -> bool {
    matches!(
        reason,
        "unsupported-transport"
            | "standard-security"
            | "unsupported-tls"
            | "missing-tunneled-tls-secret"
            | "missing-rdp-state"
            | "tls-key-update"
            | "missing-drdynvc-channel"
            | "dynamic-channel-attachment"
    )
}

/// Load and validate the pinned benchmark corpus manifest.
fn load_corpus() -> anyhow::Result<Corpus> {
    let path = project_root().join(MANIFEST_PATH);
    let contents = fs::read_to_string(&path).with_context(|| format!("read corpus manifest: {}", path.display()))?;
    parse_corpus(&contents).with_context(|| format!("validate corpus manifest: {}", path.display()))
}

fn parse_corpus(contents: &str) -> anyhow::Result<Corpus> {
    let root: toml::Table = toml::from_str(contents).context("parse TOML")?;
    ensure_allowed_keys(&root, &["upstream", "capture"], "root")?;

    let upstream = table(&root, "upstream", "root")?;
    ensure_allowed_keys(upstream, &["repository", "revision"], "upstream")?;

    let repository = string(upstream, "repository", "upstream")?;
    anyhow::ensure!(
        repository == UPSTREAM_REPOSITORY,
        "unsupported corpus repository: {repository}"
    );

    let revision = string(upstream, "revision", "upstream")?.to_owned();
    anyhow::ensure!(
        is_lower_hex(&revision, 40),
        "upstream revision must be a 40-character lowercase hexadecimal Git commit"
    );

    let captures = array(&root, "capture", "root")?;
    anyhow::ensure!(!captures.is_empty(), "corpus manifest has no captures");

    let mut ids = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut digests = BTreeSet::new();
    let mut parsed_captures = Vec::with_capacity(captures.len());

    for capture in captures {
        let capture = capture.as_table().context("capture entry must be a TOML table")?;
        ensure_allowed_keys(
            capture,
            &["id", "file", "sha256", "intent", "performance", "expect"],
            "capture",
        )?;

        let id = string(capture, "id", "capture")?.to_owned();
        anyhow::ensure!(is_identifier(&id), "invalid capture identifier: {id}");
        anyhow::ensure!(ids.insert(id.clone()), "duplicate capture identifier: {id}");

        let file = string(capture, "file", "capture")?.to_owned();
        anyhow::ensure!(is_safe_capture_file(&file), "unsafe capture file name: {file}");
        anyhow::ensure!(files.insert(file.clone()), "duplicate capture file name: {file}");

        let sha256 = string(capture, "sha256", "capture")?.to_owned();
        anyhow::ensure!(
            is_lower_hex(&sha256, 64),
            "capture digest must be a 64-character lowercase hexadecimal SHA-256: {file}"
        );
        anyhow::ensure!(digests.insert(sha256.clone()), "duplicate capture digest: {file}");

        let intent = string(capture, "intent", "capture")?.to_owned();
        anyhow::ensure!(!intent.is_empty(), "capture intent must not be empty: {file}");

        let performance = capture.get("performance").map(parse_performance_workload).transpose()?;
        let expect = parse_replay_expectation(table(capture, "expect", "capture")?)?;
        if let Some(performance) = &performance {
            anyhow::ensure!(
                expect.outcome == ReplayOutcome::Partial,
                "performance capture must have a partial replay expectation: {id}"
            );
            let summary = expect
                .summary
                .as_ref()
                .expect("partial replay expectations always have a summary");
            anyhow::ensure!(
                summary_lifecycle(summary) == Some("active"),
                "performance capture must end with an active lifecycle: {id}"
            );
            anyhow::ensure!(
                performance.criterion == format!("partial-replay/{id}/processing"),
                "invalid performance Criterion workload for capture: {id}"
            );
        }
        parsed_captures.push(Capture {
            id,
            file,
            sha256,
            intent,
            performance,
            expect,
        });
    }

    Ok(Corpus {
        revision,
        captures: parsed_captures,
    })
}

fn parse_performance_workload(value: &toml::Value) -> anyhow::Result<PerformanceWorkload> {
    let value = value.as_table().context("capture.performance must be a TOML table")?;
    ensure_allowed_keys(value, &["criterion"], "capture.performance")?;
    Ok(PerformanceWorkload {
        criterion: string(value, "criterion", "capture.performance")?.to_owned(),
    })
}

const SUMMARY_KEYS: &[&str] = &[
    "client_pdus",
    "server_pdus",
    "connection_pdus",
    "client_observation_pdus",
    "fast_path_pdus",
    "io_channel_pdus",
    "message_channel_pdus",
    "static_channel_pdus",
    "other_server_message_pdus",
    "graphics_updates",
    "final_dimensions",
    "fingerprint",
    "lifecycle",
    "framing_gaps",
    "truncated_pdu_gaps",
    "static_channel_gaps",
    "dynamic_channel_gaps",
    "session_gaps",
    "incomplete_activation_gaps",
    "unsupported_gaps",
    "gap_fingerprint",
];

fn parse_replay_expectation(value: &toml::Table) -> anyhow::Result<ReplayExpectation> {
    ensure_allowed_keys(value, &["outcome", "stage", "reason", "summary"], "capture.expect")?;
    let outcome = match string(value, "outcome", "capture.expect")? {
        "complete" => ReplayOutcome::Complete,
        "partial" => ReplayOutcome::Partial,
        "unsupported" => ReplayOutcome::Unsupported,
        outcome => anyhow::bail!("unknown capture replay outcome: {outcome}"),
    };
    let stage = optional_string(value, "stage", "capture.expect")?.map(str::to_owned);
    let reason = optional_string(value, "reason", "capture.expect")?.map(str::to_owned);
    let summary = value
        .get("summary")
        .map(|value| {
            let value = value
                .as_table()
                .context("capture.expect.summary must be a TOML table")?;
            ensure_allowed_keys(value, SUMMARY_KEYS, "capture.expect.summary")?;
            anyhow::ensure!(
                value.len() == SUMMARY_KEYS.len(),
                "capture.expect.summary must declare every replay summary field"
            );
            let mut values = std::collections::BTreeMap::new();
            for key in SUMMARY_KEYS {
                let value = value.get(*key).context("missing replay summary field")?;
                let value = match value {
                    toml::Value::Integer(value) => value.to_string(),
                    toml::Value::String(value) => value.clone(),
                    _ => anyhow::bail!("invalid replay summary field: {key}"),
                };
                values.insert((*key).to_owned(), value);
            }
            Ok(ReplaySummaryExpectation { values })
        })
        .transpose()?;
    match outcome {
        ReplayOutcome::Complete | ReplayOutcome::Partial => {
            anyhow::ensure!(
                stage.is_none() && reason.is_none(),
                "successful replay expectation cannot declare stage or reason"
            );
            anyhow::ensure!(
                summary.is_some(),
                "successful replay expectation must declare a summary"
            );
            if outcome == ReplayOutcome::Complete {
                let summary = summary.as_ref().expect("complete replay summary is required");
                anyhow::ensure!(
                    summary_lifecycle(summary) == Some("active") && !summary_has_gaps(summary),
                    "complete replay expectation must end active without gaps"
                );
            } else {
                let summary = summary.as_ref().expect("partial replay summary is required");
                anyhow::ensure!(
                    summary_lifecycle(summary) != Some("active") || summary_has_gaps(summary),
                    "partial replay expectation must have gaps or not end active"
                );
            }
        }

        ReplayOutcome::Unsupported => {
            anyhow::ensure!(
                stage.is_some() && reason.is_some(),
                "failed replay expectation must declare stage and reason"
            );
            anyhow::ensure!(summary.is_none(), "failed replay expectation cannot declare a summary");
        }
    }
    Ok(ReplayExpectation {
        outcome,
        stage,
        reason,
        summary,
    })
}

fn optional_string<'a>(table: &'a toml::Table, key: &str, location: &str) -> anyhow::Result<Option<&'a str>> {
    table
        .get(key)
        .map(|value| {
            value
                .as_str()
                .with_context(|| format!("invalid {location}.{key} string"))
        })
        .transpose()
}

fn inspect_cache(path: &Path, capture: &Capture) -> anyhow::Result<CacheState> {
    match path
        .try_exists()
        .with_context(|| format!("inspect corpus cache entry: {}", path.display()))?
    {
        false => Ok(CacheState::Missing),
        true => match verify_file(path, &capture.sha256) {
            Ok(()) => Ok(CacheState::Valid),
            Err(error) => Ok(CacheState::Corrupt(format!("{error:#}"))),
        },
    }
}

fn download(sh: &Shell, url: &str, temporary_path: &Path, expected_sha256: &str) -> anyhow::Result<()> {
    let result = cmd!(sh, "curl --fail --location --output {temporary_path} {url}")
        .run()
        .with_context(|| format!("download capture from {url}"))
        .and_then(|()| verify_file(temporary_path, expected_sha256));

    if let Err(error) = result {
        remove_partial_download(temporary_path)?;
        return Err(error);
    }

    Ok(())
}

fn remove_partial_download(path: &Path) -> anyhow::Result<()> {
    if path
        .try_exists()
        .with_context(|| format!("inspect partial download: {}", path.display()))?
    {
        fs::remove_file(path).with_context(|| format!("remove failed partial download: {}", path.display()))?;
    }

    Ok(())
}

fn verify_file(path: &Path, expected_sha256: &str) -> anyhow::Result<()> {
    let mut file = File::open(path).with_context(|| format!("open capture: {}", path.display()))?;
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest).context("read capture data")?;
    let actual_sha256 = format!("{:x}", digest.finalize());

    anyhow::ensure!(
        actual_sha256 == expected_sha256,
        "SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
    );

    Ok(())
}

fn temporary_path(cache_path: &Path) -> anyhow::Result<PathBuf> {
    let name = cache_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("cache path has no UTF-8 file name")?;
    Ok(cache_path.with_file_name(format!(".{name}.{}.part", std::process::id())))
}

fn install_capture(temporary_path: &Path, cache_path: &Path) -> anyhow::Result<()> {
    fs::rename(temporary_path, cache_path)
        .with_context(|| format!("install verified capture: {}", cache_path.display()))
}

fn format_list(corpus: &Corpus) -> String {
    corpus
        .captures
        .iter()
        .map(|capture| {
            format!(
                "{}\t{}\t{}\t{}\n",
                capture.id, capture.file, capture.sha256, capture.intent
            )
        })
        .collect()
}

fn ensure_allowed_keys(table: &toml::Table, allowed: &[&str], location: &str) -> anyhow::Result<()> {
    for key in table.keys() {
        anyhow::ensure!(allowed.contains(&key.as_str()), "unexpected key in {location}: {key}");
    }

    Ok(())
}

fn table<'a>(table: &'a toml::Table, key: &str, location: &str) -> anyhow::Result<&'a toml::Table> {
    table
        .get(key)
        .and_then(toml::Value::as_table)
        .with_context(|| format!("missing or invalid {location}.{key} table"))
}

fn array<'a>(table: &'a toml::Table, key: &str, location: &str) -> anyhow::Result<&'a Vec<toml::Value>> {
    table
        .get(key)
        .and_then(toml::Value::as_array)
        .with_context(|| format!("missing or invalid {location}.{key} array"))
}

fn string<'a>(table: &'a toml::Table, key: &str, location: &str) -> anyhow::Result<&'a str> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .with_context(|| format!("missing or invalid {location}.{key} string"))
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_safe_capture_file(value: &str) -> bool {
    value.ends_with(".pcapng")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
        && !value.starts_with('.')
        && !value.contains("..")
}

fn is_lower_hex(value: &str, expected_length: usize) -> bool {
    value.len() == expected_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

// TODO: move these implementation-detail tests to `xtask/tests` once the command surface is integration-testable.
#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    static TEST_DIRECTORY_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn corpus_toml(file: &str) -> String {
        format!(
            r#"
[upstream]
repository = "awakecoding/wireshark-rdp"
revision = "683505a753dfd7a2b27713b3a21e9a6951abacc4"

[[capture]]
id = "accepted-rdp"
file = "{file}"
sha256 = "{SHA256_ABC}"
intent = "A direct RDP session accepted by the server."
expect = {{ outcome = "unsupported", stage = "negotiate", reason = "missing-rdp-state" }}
"#
        )
    }

    fn summary(lifecycle: &str, static_channel_gaps: usize) -> ReplaySummaryExpectation {
        let values = SUMMARY_KEYS
            .iter()
            .map(|key| {
                let value = match *key {
                    "lifecycle" => lifecycle.to_owned(),
                    "static_channel_gaps" => static_channel_gaps.to_string(),
                    "final_dimensions" => "-".to_owned(),
                    "fingerprint" | "gap_fingerprint" => {
                        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned()
                    }
                    _ => "0".to_owned(),
                };
                ((*key).to_owned(), value)
            })
            .collect();
        ReplaySummaryExpectation { values }
    }

    fn test_directory() -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "ironrdp-xtask-bench-{}-{}",
            std::process::id(),
            TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        if directory.exists() {
            fs::remove_dir_all(&directory).expect("remove prior test directory");
        }
        fs::create_dir(&directory).expect("create test directory");
        directory
    }

    #[test]
    fn parses_valid_manifest() {
        let corpus = parse_corpus(&corpus_toml("accepted-rdp.pcapng")).expect("valid manifest");

        assert_eq!(corpus.captures.len(), 1);
        assert_eq!(corpus.captures[0].id, "accepted-rdp");
    }

    #[test]
    fn rejects_unobservable_replay_outcome() {
        let manifest =
            corpus_toml("accepted-rdp.pcapng").replace(r#"outcome = "unsupported""#, r#"outcome = "rejected""#);

        let error = parse_corpus(&manifest).expect_err("unobservable outcome must fail");

        assert!(error.to_string().contains("unknown capture replay outcome: rejected"));
    }

    #[test]
    fn rejects_wrong_typed_optional_replay_field() {
        let manifest = corpus_toml("accepted-rdp.pcapng").replace(r#"stage = "negotiate""#, "stage = 123");

        let error = parse_corpus(&manifest).expect_err("wrong-typed stage must fail");

        assert!(error.to_string().contains("invalid capture.expect.stage string"));
    }

    #[test]
    fn selects_manifest_qualified_capture_replay_workload() {
        let corpus =
            parse_corpus(include_str!("../../crates/ironrdp-bench/corpus.toml")).expect("valid corpus manifest");

        assert_eq!(
            capture_replay_criterion(&corpus, "no-nla-accepted").expect("qualified capture"),
            "partial-replay/no-nla-accepted/processing"
        );
        assert_eq!(
            capture_replay_criterion(&corpus, "no-nla-smartcard").expect("qualified capture"),
            "partial-replay/no-nla-smartcard/processing"
        );
    }

    #[test]
    fn rejects_unknown_capture_replay_workload_before_running_cargo() {
        let corpus = parse_corpus(&corpus_toml("accepted-rdp.pcapng")).expect("valid manifest");

        let error = capture_replay_criterion(&corpus, "typo").expect_err("unknown selector must fail");

        assert!(error.to_string().contains("unknown benchmark capture selector: typo"));
    }

    #[test]
    fn rejects_nonperformance_capture_replay_workload_before_running_cargo() {
        let corpus = parse_corpus(&corpus_toml("accepted-rdp.pcapng")).expect("valid manifest");

        let error = capture_replay_criterion(&corpus, "accepted-rdp").expect_err("ineligible selector must fail");

        assert!(
            error
                .to_string()
                .contains("capture is not eligible for a performance benchmark: accepted-rdp")
        );
    }

    #[test]
    fn rejects_unsafe_capture_file() {
        let error = parse_corpus(&corpus_toml("../capture.pcapng")).expect_err("unsafe path must fail");

        assert!(error.to_string().contains("unsafe capture file name"));
    }

    #[test]
    fn rejects_duplicate_capture_files() {
        let manifest = format!(
            "{}\n[[capture]]\nid = \"another-rdp\"\nfile = \"accepted-rdp.pcapng\"\nsha256 = \"{}\"\nintent = \"Duplicate file.\"\n",
            corpus_toml("accepted-rdp.pcapng"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );

        let error = parse_corpus(&manifest).expect_err("duplicate file must fail");

        assert!(error.to_string().contains("duplicate capture file name"));
    }

    #[test]
    fn detects_valid_and_corrupt_cache_entries() {
        let directory = test_directory();
        let path = directory.join("accepted-rdp.pcapng");
        fs::write(&path, b"abc").expect("write valid cache entry");
        let capture = parse_corpus(&corpus_toml("accepted-rdp.pcapng"))
            .expect("valid manifest")
            .captures
            .pop()
            .expect("one capture");

        assert_eq!(
            inspect_cache(&path, &capture).expect("inspect valid cache"),
            CacheState::Valid
        );

        fs::write(&path, b"corrupt").expect("corrupt cache entry");
        assert!(matches!(
            inspect_cache(&path, &capture).expect("inspect corrupt cache"),
            CacheState::Corrupt(_)
        ));

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn rejects_missing_replay_cache_entry() {
        let capture = parse_corpus(&corpus_toml("accepted-rdp.pcapng"))
            .expect("valid manifest")
            .captures
            .pop()
            .expect("one capture");
        let error = verify_file(Path::new("missing-benchmark-capture.pcapng"), &capture.sha256)
            .expect_err("missing replay cache entry must fail");

        assert!(error.to_string().contains("open capture"));
    }
    #[test]
    fn invalid_download_does_not_install_cache_entry() {
        let directory = test_directory();
        let cache_path = directory.join("accepted-rdp.pcapng");
        let partial_path = temporary_path(&cache_path).expect("temporary path");
        fs::write(&partial_path, b"wrong").expect("write partial download");

        assert!(verify_file(&partial_path, SHA256_ABC).is_err());
        remove_partial_download(&partial_path).expect("remove failed partial download");
        assert!(!cache_path.exists());
        assert!(!partial_path.exists());

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn install_replaces_an_existing_cache_entry() {
        let directory = test_directory();
        let cache_path = directory.join("accepted-rdp.pcapng");
        let temporary_path = temporary_path(&cache_path).expect("temporary path");
        fs::write(&cache_path, b"corrupt").expect("write corrupt cache entry");
        fs::write(&temporary_path, b"verified").expect("write verified temporary capture");

        install_capture(&temporary_path, &cache_path).expect("replace corrupt cache entry");

        assert_eq!(fs::read(&cache_path).expect("read replacement"), b"verified");
        assert!(!temporary_path.exists());

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn lists_manifest_entries() {
        let corpus = parse_corpus(&corpus_toml("accepted-rdp.pcapng")).expect("valid manifest");

        assert_eq!(
            format_list(&corpus),
            format!("accepted-rdp\taccepted-rdp.pcapng\t{SHA256_ABC}\tA direct RDP session accepted by the server.\n")
        );
    }

    #[test]
    fn rejects_headless_summary_with_missing_fields() {
        let error = parse_headless_summary("status=ok\tclient-pdus=1\n").expect_err("incomplete summary must fail");

        assert!(error.to_string().contains("unexpected fields"));
    }

    #[test]
    fn rejects_mismatched_unsupported_replay() {
        let capture = Capture {
            id: "accepted-rdp".to_owned(),
            file: "accepted-rdp.pcapng".to_owned(),
            sha256: SHA256_ABC.to_owned(),
            intent: "A direct RDP session accepted by the server.".to_owned(),
            performance: None,
            expect: ReplayExpectation {
                outcome: ReplayOutcome::Unsupported,
                stage: Some("negotiate".to_owned()),
                reason: Some("missing-rdp-state".to_owned()),
                summary: None,
            },
        };
        let observed = ObservedReplay::Failure {
            stage: "decrypt".to_owned(),
            reason: "standard-security".to_owned(),
        };

        let error = validate_replay_expectation(&capture, &observed).expect_err("mismatched failure must fail");

        assert!(error.to_string().contains("replay failure mismatch"));
    }

    #[test]
    fn classifies_missing_rdp_state_as_unsupported() {
        let observed = ObservedReplay::Failure {
            stage: "negotiate".to_owned(),
            reason: "missing-rdp-state".to_owned(),
        };

        assert_eq!(
            observed.outcome().expect("known limitation"),
            ReplayOutcome::Unsupported
        );
    }

    #[test]
    fn rejects_unknown_replay_failure_classification() {
        let observed = ObservedReplay::Failure {
            stage: "route".to_owned(),
            reason: "unexpected-error".to_owned(),
        };

        let error = observed.outcome().expect_err("unknown failure must not be classified");

        assert!(error.to_string().contains("unqualified headless replay failure"));
    }

    #[test]
    fn classifies_clean_never_activated_replay_as_partial() {
        let observed = ObservedReplay::Success(summary("never-activated", 0));

        assert_eq!(observed.outcome().expect("valid summary"), ReplayOutcome::Partial);
    }

    #[test]
    fn rejects_complete_expectation_with_gaps() {
        let summary = summary("active", 1);
        let values = SUMMARY_KEYS
            .iter()
            .map(|key| {
                let value = summary.values.get(*key).expect("complete summary field");
                format!("{key} = \"{value}\"")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let manifest = format!(
            r#"
outcome = "complete"
summary = {{ {values} }}
"#
        );
        let expectation = toml::from_str::<toml::Table>(&manifest).expect("valid expectation TOML");

        let error = parse_replay_expectation(&expectation).expect_err("gapped completion must fail");

        assert!(
            error
                .to_string()
                .contains("complete replay expectation must end active without gaps")
        );
    }

    #[test]
    fn rejects_partial_expectation_without_gaps() {
        let summary = summary("active", 0);
        let values = SUMMARY_KEYS
            .iter()
            .map(|key| {
                let value = summary.values.get(*key).expect("partial summary field");
                format!("{key} = \"{value}\"")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let manifest = format!(
            r#"
outcome = "partial"
summary = {{ {values} }}
"#
        );
        let expectation = toml::from_str::<toml::Table>(&manifest).expect("valid expectation TOML");

        let error = parse_replay_expectation(&expectation).expect_err("clean partial must fail");

        assert!(
            error
                .to_string()
                .contains("partial replay expectation must have gaps or not end active")
        );
    }
}
