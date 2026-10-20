//! Pinned external capture corpus.
//!
//! The manifest lives in `corpus.toml`.
//! Captures are fetched by `cargo xtask bench corpus-fetch` into `bench-data/wireshark-rdp/<revision>/captures`.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

const MANIFEST: &str = include_str!("../corpus.toml");
const UPSTREAM_REPOSITORY: &str = "awakecoding/wireshark-rdp";

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

/// One pinned capture.
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
"#;

        assert!(Corpus::parse(manifest).is_err());
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
}
