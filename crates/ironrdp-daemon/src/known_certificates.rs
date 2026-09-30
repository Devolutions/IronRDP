//! Per-user store of explicitly trusted server certificates.
//!
//! Each entry pins one server endpoint (`host:port`) to the SHA-256 fingerprint of the exact
//! certificate the user accepted. The daemon consults the store only after strict TLS validation
//! fails, so an entry is a narrow, reviewable exception rather than a blanket bypass. A changed
//! certificate no longer matches and fails again.
//!
//! The file is line-oriented, similar to SSH `known_hosts`:
//!
//! ```text
//! # comment
//! it-help-rdm:3389 3c20fd4574278531cca9c96821d5a7b9b7db9b7cbfd6c86a9f8d123ca27061a4
//! ```

use core::fmt;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use sha2::{Digest as _, Sha256};

/// Environment variable overriding the default store location.
pub const PATH_ENV: &str = "IRONRDP_AGENT_KNOWN_CERTIFICATES";

/// Port assumed when an endpoint omits one.
const DEFAULT_RDP_PORT: u16 = 3389;

const HEADER: &str = "# ironrdp-agent known certificates: <host:port> <sha256>\n";

/// SHA-256 fingerprint of a DER-encoded certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Computes the fingerprint of a DER-encoded certificate.
    #[must_use]
    pub fn of_certificate(der: &[u8]) -> Self {
        Self(Sha256::digest(der).into())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl core::str::FromStr for Fingerprint {
    type Err = anyhow::Error;

    /// Accepts 64 hex digits, optionally prefixed with `sha256:` and separated by `:` or spaces.
    fn from_str(input: &str) -> anyhow::Result<Self> {
        let input = input.trim();
        let input = input
            .strip_prefix("sha256:")
            .or_else(|| input.strip_prefix("SHA256:"))
            .unwrap_or(input);
        let digits: Vec<u8> = input.bytes().filter(|byte| !matches!(byte, b':' | b' ')).collect();
        if digits.len() != 64 {
            anyhow::bail!("expected a SHA-256 fingerprint of 64 hex digits");
        }

        let mut bytes = [0u8; 32];
        for (byte, pair) in bytes.iter_mut().zip(digits.chunks_exact(2)) {
            let pair = core::str::from_utf8(pair).context("fingerprint is not hex")?;
            *byte = u8::from_str_radix(pair, 16).context("fingerprint is not hex")?;
        }
        Ok(Self(bytes))
    }
}

/// Normalizes an endpoint to lowercase `host:port`, defaulting the port to 3389.
///
/// IPv6 addresses are bracketed, e.g. `[::1]:3389`.
pub fn normalize_endpoint(endpoint: &str) -> anyhow::Result<String> {
    let endpoint = endpoint.trim().to_ascii_lowercase();
    let (host, port) = if let Some(rest) = endpoint.strip_prefix('[') {
        let (host, rest) = rest.split_once(']').context("unterminated IPv6 address")?;
        let port = match rest {
            "" => None,
            _ => Some(rest.strip_prefix(':').context("expected `:` after IPv6 address")?),
        };
        (format!("[{host}]"), port)
    } else if endpoint.matches(':').count() > 1 {
        (format!("[{endpoint}]"), None)
    } else {
        match endpoint.split_once(':') {
            Some((host, port)) => (host.to_owned(), Some(port)),
            None => (endpoint.clone(), None),
        }
    };

    if host.is_empty() || host == "[]" {
        anyhow::bail!("endpoint host is empty");
    }
    if host.chars().any(char::is_whitespace) {
        anyhow::bail!("endpoint host contains whitespace");
    }
    let port = match port {
        Some(port) => port
            .parse::<u16>()
            .with_context(|| format!("invalid endpoint port `{port}`"))?,
        None => DEFAULT_RDP_PORT,
    };

    Ok(format!("{host}:{port}"))
}

/// Connect property accepting one certificate for that connection only, formatted like a store line
/// (`host:port <sha256>`).
pub const ACCEPT_CERTIFICATE_PROPERTY: &str = "ironrdp_accept_certificate";

/// The set of trusted certificates, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownCertificates {
    entries: Vec<(String, Fingerprint)>,
}

impl KnownCertificates {
    /// Returns the store path: `IRONRDP_AGENT_KNOWN_CERTIFICATES` when set, otherwise
    /// `%APPDATA%\ironrdp-agent\known_certificates` on Windows and
    /// `$XDG_CONFIG_HOME/ironrdp-agent/known_certificates` (or `~/.config/…`) elsewhere.
    pub fn default_path() -> anyhow::Result<PathBuf> {
        if let Some(path) = std::env::var_os(PATH_ENV).filter(|path| !path.is_empty()) {
            return Ok(PathBuf::from(path));
        }

        #[cfg(windows)]
        let root = PathBuf::from(std::env::var_os("APPDATA").context("APPDATA is not set")?);
        #[cfg(not(windows))]
        let root = match std::env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(std::env::var_os("HOME").context("neither XDG_CONFIG_HOME nor HOME is set")?)
                .join(".config"),
        };

        Ok(root.join("ironrdp-agent").join("known_certificates"))
    }

    /// Loads the store at `path`. A missing file is an empty store.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("parse {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
        }
    }

    /// Parses the store text. Every non-comment line must be a valid entry.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let mut store = Self::default();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            (|| {
                let mut fields = line.split_whitespace();
                let (Some(endpoint), Some(fingerprint), None) = (fields.next(), fields.next(), fields.next()) else {
                    anyhow::bail!("expected `<host:port> <sha256>`");
                };
                store.trust(endpoint, fingerprint.parse()?)?;
                Ok::<_, anyhow::Error>(())
            })()
            .with_context(|| format!("line {}", index + 1))?;
        }
        Ok(store)
    }

    /// Writes the store to `path`, creating its directory, and replacing the file atomically.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let directory = path.parent().filter(|parent| !parent.as_os_str().is_empty());
        if let Some(directory) = directory {
            std::fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
        }

        let mut text = String::from(HEADER);
        for (endpoint, fingerprint) in &self.entries {
            text.push_str(&format!("{endpoint} {fingerprint}\n"));
        }

        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        write_private(&temporary, text.as_bytes()).with_context(|| format!("write {}", temporary.display()))?;
        std::fs::rename(&temporary, path).with_context(|| format!("replace {}", path.display()))
    }

    /// Returns whether `der` is the certificate trusted for `endpoint`.
    #[must_use]
    pub fn is_trusted(&self, endpoint: &str, der: &[u8]) -> bool {
        let Ok(endpoint) = normalize_endpoint(endpoint) else {
            return false;
        };
        let fingerprint = Fingerprint::of_certificate(der);
        self.entries
            .iter()
            .any(|(known, trusted)| *known == endpoint && *trusted == fingerprint)
    }

    /// Trusts `fingerprint` for `endpoint`, replacing any certificate trusted for it before.
    ///
    /// Returns the normalized endpoint.
    pub fn trust(&mut self, endpoint: &str, fingerprint: Fingerprint) -> anyhow::Result<String> {
        let endpoint = normalize_endpoint(endpoint)?;
        self.entries.retain(|(known, _)| *known != endpoint);
        self.entries.push((endpoint.clone(), fingerprint));
        Ok(endpoint)
    }

    /// Removes the certificate trusted for `endpoint`, returning whether one was present.
    pub fn remove(&mut self, endpoint: &str) -> anyhow::Result<bool> {
        let endpoint = normalize_endpoint(endpoint)?;
        let before = self.entries.len();
        self.entries.retain(|(known, _)| *known != endpoint);
        Ok(self.entries.len() != before)
    }

    /// Iterates over `(endpoint, fingerprint)` entries in file order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, Fingerprint)> {
        self.entries
            .iter()
            .map(|(endpoint, fingerprint)| (endpoint.as_str(), *fingerprint))
    }
}

/// A certificate rejected by strict validation and absent from the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateRejection {
    pub endpoint: String,
    pub fingerprint: Fingerprint,
    pub reason: String,
}

impl fmt::Display for CertificateRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            endpoint,
            fingerprint,
            reason,
        } = self;
        write!(
            f,
            "untrusted certificate for {endpoint} ({reason}), SHA-256 {fingerprint}; \
             if you have verified it, trust it with `ironrdp-agent cert trust {endpoint} {fingerprint}`"
        )
    }
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINGERPRINT: &str = "3c20fd4574278531cca9c96821d5a7b9b7db9b7cbfd6c86a9f8d123ca27061a4";

    #[test]
    fn normalizes_endpoints() {
        assert_eq!(normalize_endpoint("IT-HELP-RDM").unwrap(), "it-help-rdm:3389");
        assert_eq!(normalize_endpoint("host.example:443").unwrap(), "host.example:443");
        assert_eq!(normalize_endpoint("::1").unwrap(), "[::1]:3389");
        assert_eq!(normalize_endpoint("[::1]:3390").unwrap(), "[::1]:3390");
        assert!(normalize_endpoint("host:notaport").is_err());
        assert!(normalize_endpoint("").is_err());
    }

    #[test]
    fn parses_fingerprint_formats() {
        let plain: Fingerprint = FINGERPRINT.parse().unwrap();
        let separated = FINGERPRINT
            .as_bytes()
            .chunks(2)
            .map(|pair| core::str::from_utf8(pair).unwrap().to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(":");
        assert_eq!(format!("sha256:{separated}").parse::<Fingerprint>().unwrap(), plain);
        assert_eq!(plain.to_string(), FINGERPRINT);
        assert!("abcd".parse::<Fingerprint>().is_err());
        assert!("zz".repeat(32).parse::<Fingerprint>().is_err());
    }

    #[test]
    fn trusts_only_the_pinned_certificate_for_the_endpoint() {
        let certificate = b"certificate";
        let mut store = KnownCertificates::default();
        store
            .trust("IT-HELP-RDM", Fingerprint::of_certificate(certificate))
            .unwrap();

        assert!(store.is_trusted("it-help-rdm:3389", certificate));
        assert!(!store.is_trusted("it-help-rdm:3389", b"other certificate"));
        assert!(!store.is_trusted("it-help-rdm:3390", certificate));
        assert!(!store.is_trusted("other-host:3389", certificate));
    }

    #[test]
    fn trust_replaces_and_remove_deletes() {
        let mut store = KnownCertificates::default();
        store.trust("host", Fingerprint::of_certificate(b"old")).unwrap();
        store.trust("HOST:3389", Fingerprint::of_certificate(b"new")).unwrap();
        assert_eq!(store.entries().count(), 1);
        assert!(store.is_trusted("host:3389", b"new"));

        assert!(store.remove("host").unwrap());
        assert!(!store.remove("host").unwrap());
        assert_eq!(store.entries().count(), 0);
    }

    #[test]
    fn parsing_duplicate_endpoint_keeps_only_latest_certificate() {
        let old = b"old certificate";
        let new = b"new certificate";
        let text = format!(
            "IT-HELP-RDM {} \nit-help-rdm:3389 {}\n",
            Fingerprint::of_certificate(old),
            Fingerprint::of_certificate(new)
        );
        let store = KnownCertificates::parse(&text).unwrap();
        assert_eq!(store.entries().count(), 1);
        assert!(!store.is_trusted("IT-HELP-RDM", old));
        assert!(store.is_trusted("IT-HELP-RDM", new));
    }

    #[test]
    fn round_trips_and_rejects_malformed_lines() {
        let text = format!("# comment\n\nIT-HELP-RDM {FINGERPRINT}\n");
        let store = KnownCertificates::parse(&text).unwrap();
        let reparsed = KnownCertificates::parse(&format!(
            "{HEADER}{}",
            store
                .entries()
                .map(|(endpoint, fingerprint)| format!("{endpoint} {fingerprint}\n"))
                .collect::<String>()
        ))
        .unwrap();
        assert_eq!(store, reparsed);

        let error = KnownCertificates::parse("host\n").unwrap_err();
        assert!(format!("{error:#}").contains("line 1"));
        assert!(KnownCertificates::parse(&format!("host {FINGERPRINT} extra\n")).is_err());
    }

    #[test]
    fn save_and_load_use_the_file() {
        let path = std::env::temp_dir().join(format!("ironrdp-known-certificates-{}", std::process::id()));
        assert_eq!(KnownCertificates::load(&path).unwrap(), KnownCertificates::default());

        let mut store = KnownCertificates::default();
        store.trust("host", FINGERPRINT.parse().unwrap()).unwrap();
        store.save(&path).unwrap();
        assert_eq!(KnownCertificates::load(&path).unwrap(), store);

        std::fs::remove_file(&path).unwrap();
    }
}
