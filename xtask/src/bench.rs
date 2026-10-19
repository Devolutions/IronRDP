use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::Path;

use anyhow::Context as _;
use sha2::{Digest as _, Sha256};
use xshell::{Shell, cmd};

use crate::project_root;

const MANIFEST_PATH: &str = "crates/ironrdp-bench/corpus.toml";
const CACHE_ROOT: &str = "bench-data/wireshark-rdp";
const UPSTREAM_REPOSITORY: &str = "awakecoding/wireshark-rdp";

struct Corpus {
    revision: String,
    captures: Vec<Capture>,
}

struct Capture {
    id: String,
    file: String,
    sha256: String,
    intent: String,
}

pub fn corpus_fetch(sh: &Shell) -> anyhow::Result<()> {
    let corpus = load_corpus()?;
    let cache_dir = project_root().join(CACHE_ROOT).join(&corpus.revision).join("captures");

    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("create corpus cache directory: {}", cache_dir.display()))?;

    for capture in &corpus.captures {
        let cache_path = cache_dir.join(&capture.file);

        if cache_path
            .try_exists()
            .with_context(|| format!("inspect corpus cache entry: {}", cache_path.display()))?
        {
            match verify_file(&cache_path, &capture.sha256) {
                Ok(()) => {
                    println!("Using verified cache: {}", capture.file);
                    continue;
                }
                Err(error) => println!("Repairing corrupt cache entry {}: {error:#}", capture.file),
            }
        } else {
            println!("Fetching: {}", capture.file);
        }

        let url = format!(
            "https://raw.githubusercontent.com/{UPSTREAM_REPOSITORY}/{}/captures/{}",
            corpus.revision, capture.file
        );
        let partial_path = cache_path.with_file_name(format!(".{}.{}.part", capture.file, std::process::id()));

        let result = cmd!(sh, "curl --fail --location --output {partial_path} {url}")
            .run()
            .with_context(|| format!("download capture from {url}"))
            .and_then(|()| verify_file(&partial_path, &capture.sha256))
            .and_then(|()| {
                fs::rename(&partial_path, &cache_path)
                    .with_context(|| format!("install verified capture: {}", cache_path.display()))
            });

        if let Err(error) = result {
            if let Err(cleanup_error) = fs::remove_file(&partial_path)
                && cleanup_error.kind() != std::io::ErrorKind::NotFound
            {
                println!(
                    "Failed to remove partial download {}: {cleanup_error}",
                    partial_path.display()
                );
            }
            return Err(error);
        }

        println!("Fetched and verified: {}", capture.file);
    }

    Ok(())
}

pub fn corpus_list() -> anyhow::Result<()> {
    for capture in load_corpus()?.captures {
        println!(
            "{}\t{}\t{}\t{}",
            capture.id, capture.file, capture.sha256, capture.intent
        );
    }

    Ok(())
}

fn load_corpus() -> anyhow::Result<Corpus> {
    let path = project_root().join(MANIFEST_PATH);
    let contents = fs::read_to_string(&path).with_context(|| format!("read corpus manifest: {}", path.display()))?;
    parse_corpus(&contents).with_context(|| format!("validate corpus manifest: {}", path.display()))
}

fn parse_corpus(contents: &str) -> anyhow::Result<Corpus> {
    let root: toml::Table = toml::from_str(contents).context("parse TOML")?;

    let revision = root
        .get("upstream")
        .and_then(|upstream| upstream.get("revision"))
        .and_then(toml::Value::as_str)
        .context("missing upstream.revision string")?;
    anyhow::ensure!(
        revision.len() == 40
            && revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "upstream.revision must be a 40-character lowercase hexadecimal Git commit"
    );

    let mut ids = BTreeSet::new();
    let captures = root
        .get("capture")
        .and_then(toml::Value::as_array)
        .context("missing capture array")?
        .iter()
        .map(|capture| {
            let field = |key: &str| {
                capture
                    .get(key)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .with_context(|| format!("missing capture.{key} string"))
            };
            let capture = Capture {
                id: field("id")?,
                file: field("file")?,
                sha256: field("sha256")?,
                intent: field("intent")?,
            };

            anyhow::ensure!(
                !capture.file.is_empty() && !capture.file.contains(['/', '\\']) && !capture.file.contains(".."),
                "unsafe capture file name: {}",
                capture.file
            );
            anyhow::ensure!(
                ids.insert(capture.id.clone()),
                "duplicate capture identifier: {}",
                capture.id
            );

            Ok(capture)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(Corpus {
        revision: revision.to_owned(),
        captures,
    })
}

fn verify_file(path: &Path, expected_sha256: &str) -> anyhow::Result<()> {
    let mut file = File::open(path).with_context(|| format!("open capture: {}", path.display()))?;
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest).context("read capture data")?;
    let actual_sha256 = format!("{:x}", digest.finalize());

    anyhow::ensure!(
        actual_sha256.eq_ignore_ascii_case(expected_sha256),
        "SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pinned_manifest() {
        let corpus = parse_corpus(include_str!("../../crates/ironrdp-bench/corpus.toml")).expect("valid manifest");

        assert_eq!(corpus.captures.len(), 27);
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

        assert!(parse_corpus(manifest).is_err());
    }

    #[test]
    fn detects_corrupt_cache_entry() {
        const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let path = std::env::temp_dir().join(format!("ironrdp-xtask-corpus-{}.pcapng", std::process::id()));

        fs::write(&path, b"abc").expect("write valid cache entry");
        assert!(verify_file(&path, SHA256_ABC).is_ok());

        fs::write(&path, b"corrupt").expect("corrupt cache entry");
        assert!(verify_file(&path, SHA256_ABC).is_err());

        fs::remove_file(&path).expect("remove test cache entry");
    }
}
