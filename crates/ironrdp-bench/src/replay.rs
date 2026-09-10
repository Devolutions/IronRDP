//! Prepared, qualified partial-replay workloads shared by CLI and Criterion.

use core::str::FromStr;
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};

use ironrdp_capture_replay::{
    ReplayExecution, ReplayLifecycle, ReplayOptions, ReplaySummary, prepare_capture, read_capture,
};
use sha2::{Digest as _, Sha256};

const MANIFEST: &str = include_str!("../corpus.toml");
const CACHE_ROOT: &str = "bench-data/wireshark-rdp";

/// A replay workload with useful rendered output and known, qualified gaps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartialReplayId {
    /// The no-NLA accepted capture.
    NoNlaAccepted,
    /// The no-NLA smartcard capture.
    NoNlaSmartcard,
}

impl PartialReplayId {
    /// All partial replay workloads suitable for processing benchmarks.
    pub const ALL: [Self; 2] = [Self::NoNlaAccepted, Self::NoNlaSmartcard];

    /// Stable workload identifier shared by the CLI and Criterion.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoNlaAccepted => "no-nla-accepted",
            Self::NoNlaSmartcard => "no-nla-smartcard",
        }
    }
}

impl FromStr for PartialReplayId {
    type Err = ReplayWorkloadError;

    fn from_str(value: &str) -> ReplayWorkloadResult<Self> {
        match value {
            "no-nla-accepted" => Ok(Self::NoNlaAccepted),
            "no-nla-smartcard" => Ok(Self::NoNlaSmartcard),
            _ => Err(ReplayWorkloadError::new(format!(
                "unknown partial replay workload: {value}"
            ))),
        }
    }
}

/// A prepared capture replay with a manifest-qualified output contract.
pub struct PartialReplayWorkload {
    id: PartialReplayId,
    expected: ReplaySummary,
    prepared: ironrdp_capture_replay::PreparedReplay,
}

impl PartialReplayWorkload {
    /// Verify, read, decrypt, and prepare the selected cached capture.
    ///
    /// This intentionally runs outside focused Criterion timing.
    pub fn prepare(id: PartialReplayId) -> ReplayWorkloadResult<Self> {
        let cache_root = project_root().join(CACHE_ROOT);
        let capture = expected_capture(id)?;
        let path = cache_root.join(&capture.revision).join("captures").join(&capture.file);
        verify_file(&path, &capture.sha256)?;

        let capture_data = read_capture(&path)
            .map_err(|error| ReplayWorkloadError::new(format!("read capture {}: {error}", path.display())))?;
        let prepared = prepare_capture(&capture_data)
            .map_err(|error| ReplayWorkloadError::new(format!("prepare capture {}: {error}", path.display())))?;

        Ok(Self {
            id,
            expected: capture.expected,
            prepared,
        })
    }

    /// Return the verified local path for one manifest-qualified partial replay.
    ///
    /// This is shared by offline benchmark workloads that need the original capture
    /// in addition to the prepared passive replay.
    pub(crate) fn cached_capture_path(id: PartialReplayId) -> ReplayWorkloadResult<PathBuf> {
        let capture = expected_capture(id)?;
        let path = project_root()
            .join(CACHE_ROOT)
            .join(capture.revision)
            .join("captures")
            .join(capture.file);
        verify_file(&path, &capture.sha256)?;
        Ok(path)
    }

    /// Execute one strict replay and enforce its complete partial-replay contract.
    ///
    /// This includes the full framebuffer output fingerprint and is intended for
    /// preflight verification or a standalone one-run measurement.
    pub fn verify(&self) -> ReplayWorkloadResult<ReplayMeasurement> {
        self.execute(true)
    }

    /// Execute one fresh replay and enforce its processing contract.
    ///
    /// This starts with fresh session state and verifies lifecycle, routing,
    /// gap, and graphics-update boundaries without hashing every framebuffer.
    /// Call [`Self::verify`] before using this in a focused benchmark.
    pub fn replay(&self) -> ReplayWorkloadResult<ReplayMeasurement> {
        self.execute(false)
    }

    fn execute(&self, calculate_output_fingerprint: bool) -> ReplayWorkloadResult<ReplayMeasurement> {
        let execution = self
            .prepared
            .replay_with_options(ReplayOptions {
                calculate_output_fingerprint,
            })
            .map_err(|error| ReplayWorkloadError::new(format!("replay {}: {error}", self.id.as_str())))?;
        validate_execution(&execution, &self.expected, calculate_output_fingerprint)?;

        Ok(ReplayMeasurement {
            routed_pdus: execution.report.events.len(),
            graphics_updates: execution.summary.graphics_updates,
        })
    }

    /// Stable workload identifier.
    pub const fn id(&self) -> PartialReplayId {
        self.id
    }
}

/// Payload-free observations from one qualified replay execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayMeasurement {
    /// Number of routed client and server PDUs.
    pub routed_pdus: usize,
    /// Number of rendered graphics updates.
    pub graphics_updates: usize,
}

/// Error returned when a cached workload cannot satisfy its declared contract.
#[derive(Debug)]
pub struct ReplayWorkloadError(String);

impl ReplayWorkloadError {
    fn new(message: String) -> Self {
        Self(message)
    }
}

impl fmt::Display for ReplayWorkloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for ReplayWorkloadError {}

/// Result returned by prepared replay workload operations.
pub type ReplayWorkloadResult<T> = Result<T, ReplayWorkloadError>;

struct ExpectedCapture {
    revision: String,
    file: String,
    sha256: String,
    expected: ReplaySummary,
}

fn expected_capture(id: PartialReplayId) -> ReplayWorkloadResult<ExpectedCapture> {
    expected_capture_from_manifest(MANIFEST, id)
}

fn expected_capture_from_manifest(manifest: &str, id: PartialReplayId) -> ReplayWorkloadResult<ExpectedCapture> {
    let manifest: toml::Table = toml::from_str(manifest)
        .map_err(|error| ReplayWorkloadError::new(format!("parse corpus manifest: {error}")))?;
    let upstream = required_table(&manifest, "upstream", "root")?;
    let revision = required_string(upstream, "revision", "upstream")?.to_owned();
    if !is_lower_hex(&revision, 40) {
        return Err(ReplayWorkloadError::new(
            "corpus manifest has an invalid upstream revision".to_owned(),
        ));
    }
    let captures = required_array(&manifest, "capture", "root")?;
    let capture = captures
        .iter()
        .filter_map(toml::Value::as_table)
        .find(|capture| capture.get("id").and_then(toml::Value::as_str) == Some(id.as_str()))
        .ok_or_else(|| ReplayWorkloadError::new(format!("missing {} corpus entry", id.as_str())))?;
    let expected = required_table(capture, "expect", "capture")?;
    let outcome = required_string(expected, "outcome", "capture.expect")?;
    if outcome != "partial" {
        return Err(ReplayWorkloadError::new(format!(
            "{} must remain a qualified partial replay",
            id.as_str()
        )));
    }

    let file = required_string(capture, "file", "capture")?.to_owned();
    if !is_safe_capture_file(&file) {
        return Err(ReplayWorkloadError::new(format!("unsafe capture file name: {file}")));
    }
    let sha256 = required_string(capture, "sha256", "capture")?.to_owned();
    parse_digest(&sha256)?;

    Ok(ExpectedCapture {
        revision,
        file,
        sha256,
        expected: parse_expectation(required_table(expected, "summary", "capture.expect")?)?,
    })
}

fn parse_expectation(summary: &toml::Table) -> ReplayWorkloadResult<ReplaySummary> {
    Ok(ReplaySummary {
        client_pdus: required_usize(summary, "client_pdus")?,
        server_pdus: required_usize(summary, "server_pdus")?,
        connection_pdus: required_usize(summary, "connection_pdus")?,
        client_observation_pdus: required_usize(summary, "client_observation_pdus")?,
        fast_path_pdus: required_usize(summary, "fast_path_pdus")?,
        io_channel_pdus: required_usize(summary, "io_channel_pdus")?,
        message_channel_pdus: required_usize(summary, "message_channel_pdus")?,
        static_channel_pdus: required_usize(summary, "static_channel_pdus")?,
        other_server_message_pdus: required_usize(summary, "other_server_message_pdus")?,
        graphics_updates: required_usize(summary, "graphics_updates")?,
        final_dimensions: parse_dimensions(required_string(summary, "final_dimensions", "capture.expect.summary")?)?,
        output_fingerprint: Some(parse_digest(required_string(
            summary,
            "fingerprint",
            "capture.expect.summary",
        )?)?),
        lifecycle: parse_lifecycle(required_string(summary, "lifecycle", "capture.expect.summary")?)?,
        framing_gaps: required_usize(summary, "framing_gaps")?,
        truncated_pdu_gaps: required_usize(summary, "truncated_pdu_gaps")?,
        static_channel_gaps: required_usize(summary, "static_channel_gaps")?,
        dynamic_channel_gaps: required_usize(summary, "dynamic_channel_gaps")?,
        session_gaps: required_usize(summary, "session_gaps")?,
        incomplete_activation_gaps: required_usize(summary, "incomplete_activation_gaps")?,
        unsupported_gaps: required_usize(summary, "unsupported_gaps")?,
        gap_fingerprint: parse_digest(required_string(summary, "gap_fingerprint", "capture.expect.summary")?)?,
    })
}

fn validate_execution(
    execution: &ReplayExecution,
    expected: &ReplaySummary,
    verify_output_fingerprint: bool,
) -> ReplayWorkloadResult<()> {
    let mut expected = expected.clone();
    if !verify_output_fingerprint {
        expected.output_fingerprint = None;
    }
    if execution.summary != expected
        || execution.report.lifecycle != expected.lifecycle
        || execution.report.events.len()
            != execution
                .summary
                .client_pdus
                .checked_add(execution.summary.server_pdus)
                .ok_or_else(|| ReplayWorkloadError::new("replay PDU count overflows usize".to_owned()))?
        || execution.report.gaps.len() != gap_count(&execution.summary)?
    {
        return Err(ReplayWorkloadError::new(
            "partial replay no longer matches its declared contract".to_owned(),
        ));
    }
    Ok(())
}

fn gap_count(summary: &ReplaySummary) -> ReplayWorkloadResult<usize> {
    [
        summary.framing_gaps,
        summary.truncated_pdu_gaps,
        summary.static_channel_gaps,
        summary.dynamic_channel_gaps,
        summary.session_gaps,
        summary.incomplete_activation_gaps,
        summary.unsupported_gaps,
    ]
    .into_iter()
    .try_fold(0usize, |count, gaps| {
        count
            .checked_add(gaps)
            .ok_or_else(|| ReplayWorkloadError::new("replay gap count overflows usize".to_owned()))
    })
}

fn verify_file(path: &Path, expected: &str) -> ReplayWorkloadResult<()> {
    let mut file = File::open(path)
        .map_err(|error| ReplayWorkloadError::new(format!("open capture {}: {error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)
            .map_err(|error| ReplayWorkloadError::new(format!("hash capture {}: {error}", path.display())))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual: [u8; 32] = hasher.finalize().into();
    if actual != parse_digest(expected)? {
        return Err(ReplayWorkloadError::new(format!(
            "SHA-256 mismatch for capture {}",
            path.display()
        )));
    }
    Ok(())
}

fn required_table<'a>(table: &'a toml::Table, key: &str, location: &str) -> ReplayWorkloadResult<&'a toml::Table> {
    table
        .get(key)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| ReplayWorkloadError::new(format!("missing or invalid {location}.{key} table")))
}

fn required_array<'a>(table: &'a toml::Table, key: &str, location: &str) -> ReplayWorkloadResult<&'a Vec<toml::Value>> {
    table
        .get(key)
        .and_then(toml::Value::as_array)
        .ok_or_else(|| ReplayWorkloadError::new(format!("missing or invalid {location}.{key} array")))
}

fn required_string<'a>(table: &'a toml::Table, key: &str, location: &str) -> ReplayWorkloadResult<&'a str> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .ok_or_else(|| ReplayWorkloadError::new(format!("missing or invalid {location}.{key} string")))
}

fn required_usize(table: &toml::Table, key: &str) -> ReplayWorkloadResult<usize> {
    let value = table
        .get(key)
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| ReplayWorkloadError::new(format!("missing or invalid capture.expect.summary.{key} integer")))?;
    usize::try_from(value)
        .map_err(|_| ReplayWorkloadError::new(format!("invalid capture.expect.summary.{key} integer")))
}

fn parse_dimensions(value: &str) -> ReplayWorkloadResult<Option<(u16, u16)>> {
    if value == "-" {
        return Ok(None);
    }
    let (width, height) = value
        .split_once('x')
        .ok_or_else(|| ReplayWorkloadError::new("invalid capture.expect.summary.final_dimensions".to_owned()))?;
    Ok(Some((
        width
            .parse()
            .map_err(|_| ReplayWorkloadError::new("invalid capture.expect.summary.final_dimensions".to_owned()))?,
        height
            .parse()
            .map_err(|_| ReplayWorkloadError::new("invalid capture.expect.summary.final_dimensions".to_owned()))?,
    )))
}

fn parse_lifecycle(value: &str) -> ReplayWorkloadResult<ReplayLifecycle> {
    match value {
        "never-activated" => Ok(ReplayLifecycle::NeverActivated),
        "active" => Ok(ReplayLifecycle::Active),
        "deactivated" => Ok(ReplayLifecycle::Deactivated),
        _ => Err(ReplayWorkloadError::new(
            "invalid capture.expect.summary.lifecycle".to_owned(),
        )),
    }
}

fn parse_digest(value: &str) -> ReplayWorkloadResult<[u8; 32]> {
    let mut digest = [0; 32];
    if value.len() != digest.len() * 2 {
        return Err(ReplayWorkloadError::new("invalid SHA-256 digest".to_owned()));
    }
    for (byte, pair) in digest.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let high = match pair[0] {
            b'0'..=b'9' => pair[0] - b'0',
            b'a'..=b'f' => pair[0] - b'a' + 10,
            _ => return Err(ReplayWorkloadError::new("invalid SHA-256 digest".to_owned())),
        };
        let low = match pair[1] {
            b'0'..=b'9' => pair[1] - b'0',
            b'a'..=b'f' => pair[1] - b'a' + 10,
            _ => return Err(ReplayWorkloadError::new("invalid SHA-256 digest".to_owned())),
        };
        *byte = (high << 4) | low;
    }
    Ok(digest)
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

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("ironrdp-bench must be nested under the workspace crates directory")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_partial_replay_id() {
        let error = "routing-only"
            .parse::<PartialReplayId>()
            .expect_err("unknown workload must fail");
        assert_eq!(error.to_string(), "unknown partial replay workload: routing-only");
    }

    #[test]
    fn loads_qualified_rendered_workload_contracts() {
        let accepted = expected_capture(PartialReplayId::NoNlaAccepted).expect("accepted workload is declared");
        let smartcard = expected_capture(PartialReplayId::NoNlaSmartcard).expect("smartcard workload is declared");

        assert_eq!(accepted.expected.graphics_updates, 78);
        assert_eq!(smartcard.expected.graphics_updates, 79);
        assert_eq!(accepted.expected.lifecycle, ReplayLifecycle::Active);
        assert_eq!(smartcard.expected.lifecycle, ReplayLifecycle::Active);
        assert_eq!(accepted.expected.static_channel_gaps, 8);
        assert_eq!(smartcard.expected.static_channel_gaps, 8);
    }

    #[test]
    fn declares_every_manifest_qualified_workload() {
        let manifest: toml::Table = toml::from_str(MANIFEST).expect("valid corpus manifest");
        let captures = required_array(&manifest, "capture", "root").expect("capture entries");
        let manifest_ids = captures
            .iter()
            .filter_map(toml::Value::as_table)
            .filter(|capture| capture.contains_key("performance"))
            .map(|capture| required_string(capture, "id", "capture").expect("performance capture id"))
            .collect::<Vec<_>>();

        assert_eq!(manifest_ids, PartialReplayId::ALL.map(PartialReplayId::as_str));
    }

    #[test]
    fn rejects_complete_capture_as_partial_workload() {
        let manifest = MANIFEST
            .replacen("id = \"clipboard-various-formats\"", "id = \"no-nla-accepted\"", 1)
            .replacen("outcome = \"partial\"", "outcome = \"complete\"", 1);
        assert!(expected_capture_from_manifest(&manifest, PartialReplayId::NoNlaAccepted).is_err());
    }

    #[test]
    fn rejects_mismatched_replay_summary() {
        let expected = expected_capture(PartialReplayId::NoNlaAccepted)
            .expect("accepted workload is declared")
            .expected;
        let execution = ReplayExecution {
            report: Default::default(),
            summary: ReplaySummary::default(),
        };
        assert!(validate_execution(&execution, &expected, true).is_err());
    }
}
