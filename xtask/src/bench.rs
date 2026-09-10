use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use sha2::{Digest as _, Sha256};
use xshell::{Shell, cmd};

const MANIFEST_PATH: &str = "crates/ironrdp-bench/corpus.toml";
const CACHE_ROOT: &str = "dependencies/wireshark-rdp";
const UPSTREAM_REPOSITORY: &str = "awakecoding/wireshark-rdp";

#[derive(Debug, Eq, PartialEq)]
struct Corpus {
    revision: String,
    captures: Vec<Capture>,
}

#[derive(Debug, Eq, PartialEq)]
struct Capture {
    id: String,
    file: String,
    sha256: String,
    intent: String,
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
        ensure_allowed_keys(capture, &["id", "file", "sha256", "intent"], "capture")?;

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

        parsed_captures.push(Capture {
            id,
            file,
            sha256,
            intent,
        });
    }

    Ok(Corpus {
        revision,
        captures: parsed_captures,
    })
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
        .with_context(|| format!("download capture from {url}"));

    if let Err(error) = result {
        remove_partial_download(temporary_path)?;
        return Err(error);
    }

    if let Err(error) = verify_file(temporary_path, expected_sha256) {
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
    let actual_sha256 = copy_and_hash(&mut file, &mut std::io::sink())?;

    anyhow::ensure!(
        actual_sha256 == expected_sha256,
        "SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
    );

    Ok(())
}

fn copy_and_hash(reader: &mut dyn std::io::Read, writer: &mut dyn std::io::Write) -> anyhow::Result<String> {
    let mut buffer = [0; 64 * 1024];
    let mut digest = Sha256::new();

    loop {
        let bytes_read = reader.read(&mut buffer).context("read capture data")?;
        if bytes_read == 0 {
            break;
        }

        digest.update(&buffer[..bytes_read]);
        writer.write_all(&buffer[..bytes_read]).context("write capture data")?;
    }

    Ok(format!("{:x}", digest.finalize()))
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
    let mut list = String::new();

    for capture in &corpus.captures {
        list.push_str(&capture.id);
        list.push('\t');
        list.push_str(&capture.file);
        list.push('\t');
        list.push_str(&capture.sha256);
        list.push('\t');
        list.push_str(&capture.intent);
        list.push('\n');
    }

    list
}

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest directory has no parent")
        .to_path_buf()
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
        && value.len() > ".pcapng".len()
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
"#
        )
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
}
