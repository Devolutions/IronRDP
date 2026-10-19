//! Pinned external capture corpus and capture replay expectations.
//!
//! The manifest lives in `corpus.toml`.
//! Captures are fetched by `cargo xtask bench corpus-fetch` into `bench-data/wireshark-rdp/<revision>/captures`.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;
use ironrdp_capture::read_capture;
use ironrdp_capture_replay::{
    ReplayDirection, ReplayError, ReplayGap, ReplayLifecycle, ReplayOptions, ReplayOutcome, ReplaySummary,
    prepare_capture,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const MANIFEST: &str = include_str!("../corpus.toml");
const UPSTREAM_REPOSITORY: &str = "awakecoding/wireshark-rdp";
const MAX_GAP_DETAILS: usize = 16;

/// Pinned capture corpus manifest.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    pub upstream: Upstream,
    /// Captures in manifest order.
    #[serde(rename = "capture")]
    pub captures: Vec<CorpusCapture>,
}

/// Pinned upstream source of the capture files.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upstream {
    /// Immutable upstream Git commit.
    pub revision: String,
}

/// One pinned capture and its expected replay result.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorpusCapture {
    /// Stable manifest identifier.
    pub id: String,
    pub file: String,
    /// Expected hexadecimal SHA-256 digest.
    pub sha256: String,
    /// Scenario recorded by the upstream capture.
    pub intent: String,
    /// Benchmark kinds that use this capture.
    #[serde(default)]
    pub benchmark: Vec<String>,
    /// Expected capture replay result.
    pub expect: Expectation,
}

/// Expected capture replay result, tagged by its outcome.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Expectation {
    /// The session activated and replayed without gaps.
    Complete(Observation),
    /// The session activated, but replay recorded gaps or ended deactivated.
    Partial(Observation),
    /// A documented replay limitation prevented replay.
    Unsupported(Limitation),
}

/// Exact payload-free result of a replay that reached activation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    /// Activation state when routing stopped.
    pub lifecycle: Lifecycle,
    /// Client PDUs consumed.
    pub client_pdus: usize,
    /// Server connection and activation PDUs.
    pub connection_pdus: usize,
    /// Server Fast-Path PDUs.
    pub fast_path_pdus: usize,
    /// Server I/O channel PDUs.
    pub io_channel_pdus: usize,
    /// Server message-channel PDUs.
    pub message_channel_pdus: usize,
    /// Server static-channel PDUs.
    pub static_channel_pdus: usize,
    /// Server MCS PDUs outside the captured active channels.
    pub other_server_message_pdus: usize,
    /// Composited framebuffer updates.
    pub graphics_updates: usize,
    /// Width and height of the final framebuffer update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_dimensions: Option<[u16; 2]>,
    /// Recorded gaps.
    pub gaps: usize,
    /// SHA-256 over ordered framebuffer updates.
    pub fingerprint: String,
    /// SHA-256 over ordered gap metadata.
    pub gap_fingerprint: String,
}

/// Documented replay limitation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limitation {
    /// Stable reason code reported by `ironrdp-capture-replay`.
    pub reason: String,
}

/// Activation state when routing stopped.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    /// The capture never reached activation.
    NeverActivated,
    /// The capture ended with an active session.
    Active,
    /// The capture ended after a deactivation without a completed reactivation.
    Deactivated,
    /// The session terminated, and routing stopped there.
    Terminated,
}

impl Corpus {
    /// Parse and validate the pinned manifest.
    pub fn load() -> anyhow::Result<Self> {
        Self::parse(MANIFEST).context("validate crates/ironrdp-bench/corpus.toml")
    }

    fn parse(contents: &str) -> anyhow::Result<Self> {
        let corpus: Self = toml::from_str(contents)?;
        anyhow::ensure!(
            corpus.upstream.revision.len() == 40
                && corpus
                    .upstream
                    .revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "upstream.revision must be a 40-character lowercase hexadecimal Git commit"
        );
        let mut ids = BTreeSet::new();
        for capture in &corpus.captures {
            anyhow::ensure!(ids.insert(&capture.id), "duplicate capture identifier: {}", capture.id);
            anyhow::ensure!(
                !capture.file.is_empty() && !capture.file.contains(['/', '\\']) && !capture.file.contains(".."),
                "unsafe capture file name: {}",
                capture.file
            );
        }
        Ok(corpus)
    }

    /// Directory containing the verified cached captures for the pinned revision.
    pub fn cache_dir(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../bench-data/wireshark-rdp")
            .join(&self.upstream.revision)
            .join("captures")
    }
}

impl Expectation {
    /// Build the expectation that a replay summary satisfies.
    pub fn from_summary(summary: &ReplaySummary) -> Self {
        let observation = || Observation {
            lifecycle: match summary.lifecycle {
                ReplayLifecycle::NeverActivated => Lifecycle::NeverActivated,
                ReplayLifecycle::Active => Lifecycle::Active,
                ReplayLifecycle::Deactivated => Lifecycle::Deactivated,
                ReplayLifecycle::Terminated => Lifecycle::Terminated,
            },
            client_pdus: summary.client_pdus,
            connection_pdus: summary.connection_pdus,
            fast_path_pdus: summary.fast_path_pdus,
            io_channel_pdus: summary.io_channel_pdus,
            message_channel_pdus: summary.message_channel_pdus,
            static_channel_pdus: summary.static_channel_pdus,
            other_server_message_pdus: summary.other_server_message_pdus,
            graphics_updates: summary.graphics_updates,
            final_dimensions: summary.final_dimensions.map(|(width, height)| [width, height]),
            gaps: summary.gaps,
            fingerprint: summary.output_fingerprint.clone().unwrap_or_default(),
            gap_fingerprint: summary.gap_fingerprint.clone(),
        };
        match summary.outcome() {
            ReplayOutcome::Complete => Self::Complete(observation()),
            ReplayOutcome::Partial => Self::Partial(observation()),
            ReplayOutcome::Unsupported(reason) => Self::Unsupported(Limitation {
                reason: reason.to_owned(),
            }),
        }
    }

    fn outcome(&self) -> &'static str {
        match self {
            Self::Complete(_) => "complete",
            Self::Partial(_) => "partial",
            Self::Unsupported(_) => "unsupported",
        }
    }
}

/// Download missing or corrupt captures from the pinned upstream commit and verify them.
///
/// This is the only corpus operation that uses the network; it requires `curl` on `PATH`.
pub fn fetch(out: &mut dyn Write) -> anyhow::Result<()> {
    let corpus = Corpus::load()?;
    let cache_dir = corpus.cache_dir();
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("create corpus cache directory: {}", cache_dir.display()))?;

    for capture in &corpus.captures {
        let cache_path = cache_dir.join(&capture.file);

        if cache_path
            .try_exists()
            .with_context(|| format!("inspect corpus cache entry: {}", cache_path.display()))?
        {
            match verify_digest(&cache_path, &capture.sha256) {
                Ok(()) => {
                    writeln!(out, "Using verified cache: {}", capture.file)?;
                    continue;
                }
                Err(error) => writeln!(out, "Repairing corrupt cache entry {}: {error:#}", capture.file)?,
            }
        } else {
            writeln!(out, "Fetching: {}", capture.file)?;
        }

        let url = format!(
            "https://raw.githubusercontent.com/{UPSTREAM_REPOSITORY}/{}/captures/{}",
            corpus.upstream.revision, capture.file
        );
        let partial_path = cache_dir.join(format!(".{}.{}.part", capture.file, std::process::id()));

        let result = download(&url, &partial_path)
            .and_then(|()| verify_digest(&partial_path, &capture.sha256))
            .and_then(|()| {
                fs::rename(&partial_path, &cache_path)
                    .with_context(|| format!("install verified capture: {}", cache_path.display()))
            });

        if let Err(error) = result {
            if let Err(cleanup_error) = fs::remove_file(&partial_path)
                && cleanup_error.kind() != std::io::ErrorKind::NotFound
            {
                writeln!(
                    out,
                    "Failed to remove partial download {}: {cleanup_error}",
                    partial_path.display()
                )?;
            }
            return Err(error);
        }

        writeln!(out, "Fetched and verified: {}", capture.file)?;
    }

    Ok(())
}

/// Write one tab-separated line per capture: identifier, file name, digest, and intent.
pub fn list(out: &mut dyn Write) -> anyhow::Result<()> {
    for capture in Corpus::load()?.captures {
        writeln!(
            out,
            "{}\t{}\t{}\t{}",
            capture.id, capture.file, capture.sha256, capture.intent
        )?;
    }
    Ok(())
}

/// Replay cached captures with full verification and compare every result with its expectation.
///
/// Prints the observed expectation as TOML for each mismatch and fails if any capture mismatched or could not be verified.
pub fn check(selector: Option<&str>, out: &mut dyn Write) -> anyhow::Result<()> {
    let corpus = Corpus::load()?;
    let cache_dir = corpus.cache_dir();
    let captures = corpus
        .captures
        .iter()
        .filter(|capture| selector.is_none_or(|id| capture.id == id))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !captures.is_empty(),
        "unknown capture: {}",
        selector.unwrap_or_default()
    );

    let mut counts = [0usize; 3];
    let mut failures = 0usize;
    for capture in captures {
        let (observed, gaps) = match observe(&cache_dir.join(&capture.file), &capture.sha256) {
            Ok(observed) => observed,
            Err(error) => {
                failures += 1;
                writeln!(out, "{}: error: {error:#}", capture.id)?;
                continue;
            }
        };
        counts[match observed {
            Expectation::Complete(_) => 0,
            Expectation::Partial(_) => 1,
            Expectation::Unsupported(_) => 2,
        }] += 1;
        if observed == capture.expect {
            writeln!(out, "{}: {}", capture.id, observed.outcome())?;
            continue;
        }

        failures += 1;
        writeln!(
            out,
            "{}: mismatch: expected {}, observed {}; replace its expectation with:\n[capture.expect]\n{}",
            capture.id,
            capture.expect.outcome(),
            observed.outcome(),
            toml::to_string(&observed)?
        )?;
        for gap in gaps.iter().take(MAX_GAP_DETAILS) {
            let direction = match gap.direction {
                ReplayDirection::Client => "client",
                ReplayDirection::Server => "server",
            };
            writeln!(
                out,
                "  gap: packet {}, {direction}, {:?}, skipped {} byte(s)",
                gap.packet, gap.kind, gap.skipped_bytes
            )?;
        }
        if gaps.len() > MAX_GAP_DETAILS {
            writeln!(out, "  {} more gap(s)", gaps.len() - MAX_GAP_DETAILS)?;
        }
    }

    let [complete, partial, unsupported] = counts;
    writeln!(
        out,
        "Capture outcomes: complete={complete}, partial={partial}, unsupported={unsupported}"
    )?;
    anyhow::ensure!(failures == 0, "{failures} capture(s) failed verification");
    Ok(())
}

fn observe(path: &Path, sha256: &str) -> anyhow::Result<(Expectation, Vec<ReplayGap>)> {
    verify_digest(path, sha256).context("verify cached capture")?;

    let result = read_capture(path)
        .map_err(ReplayError::from)
        .and_then(|capture| prepare_capture(&capture))
        .and_then(|prepared| {
            prepared.replay_with_options(ReplayOptions {
                calculate_output_fingerprint: true,
            })
        });
    match result {
        Ok(execution) => Ok((Expectation::from_summary(&execution.summary), execution.report.gaps)),
        Err(error) => match error.unsupported_reason() {
            Some(reason) => Ok((
                Expectation::Unsupported(Limitation {
                    reason: reason.to_owned(),
                }),
                Vec::new(),
            )),
            None => Err(anyhow::Error::new(error).context("replay capture")),
        },
    }
}

fn download(url: &str, path: &Path) -> anyhow::Result<()> {
    let status = Command::new("curl")
        .args(["--fail", "--location", "--silent", "--show-error", "--output"])
        .arg(path)
        .arg(url)
        .status()
        .context("run curl")?;
    anyhow::ensure!(status.success(), "download capture from {url}: curl {status}");
    Ok(())
}

fn verify_digest(path: &Path, sha256: &str) -> anyhow::Result<()> {
    let mut file = File::open(path).with_context(|| format!("open capture: {}", path.display()))?;
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest).context("read capture")?;
    let actual = format!("{:x}", digest.finalize());
    anyhow::ensure!(
        actual.eq_ignore_ascii_case(sha256),
        "SHA-256 mismatch: expected {sha256}, got {actual}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pinned_manifest() {
        assert_eq!(Corpus::load().expect("valid manifest").captures.len(), 27);
    }

    #[test]
    fn rejects_traversal_file_name() {
        let manifest = r#"
[upstream]
revision = "683505a753dfd7a2b27713b3a21e9a6951abacc4"

[[capture]]
id = "escape"
file = "../escape.pcapng"
sha256 = ""
intent = ""
expect = { outcome = "unsupported", reason = "test" }
"#;

        let error = Corpus::parse(manifest).expect_err("traversal must fail");
        assert!(error.to_string().contains("unsafe capture file name"));
    }

    #[test]
    fn detects_corrupt_cache_entry() {
        const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let path = std::env::temp_dir().join(format!("ironrdp-bench-corpus-{}.pcapng", std::process::id()));

        fs::write(&path, b"abc").expect("write valid cache entry");
        assert!(verify_digest(&path, SHA256_ABC).is_ok());

        fs::write(&path, b"corrupt").expect("corrupt cache entry");
        assert!(verify_digest(&path, SHA256_ABC).is_err());

        fs::remove_file(&path).expect("remove test cache entry");
    }

    #[test]
    fn compares_typed_expectation_with_summary() {
        let summary = ReplaySummary {
            lifecycle: ReplayLifecycle::Terminated,
            client_pdus: 3,
            fast_path_pdus: 2,
            graphics_updates: 1,
            final_dimensions: Some((32, 16)),
            output_fingerprint: Some("ab".to_owned()),
            gaps: 1,
            gap_fingerprint: "cd".to_owned(),
            ..ReplaySummary::default()
        };
        let expected: Expectation = toml::from_str(
            r#"
outcome = "partial"
lifecycle = "terminated"
client_pdus = 3
connection_pdus = 0
fast_path_pdus = 2
io_channel_pdus = 0
message_channel_pdus = 0
static_channel_pdus = 0
other_server_message_pdus = 0
graphics_updates = 1
final_dimensions = [32, 16]
gaps = 1
fingerprint = "ab"
gap_fingerprint = "cd"
"#,
        )
        .expect("valid expectation");

        assert_eq!(Expectation::from_summary(&summary), expected);
        assert!(toml::from_str::<Expectation>("outcome = \"unsupported\"\nreason = \"x\"\nstage = \"y\"").is_err());
    }
}
