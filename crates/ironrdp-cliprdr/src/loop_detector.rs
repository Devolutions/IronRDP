//! Clipboard synchronization loop detection.
//!
//! An embedder bridging CLIPRDR to a local OS clipboard commonly hits a
//! feedback loop: content copied on one side is synced to the other, the
//! other side's own change notification then fires for that same content,
//! and the embedder syncs it back, forever. `ironrdp-cliprdr` has no
//! visibility into the local OS clipboard on either side (that lives
//! entirely in the [`crate::backend::CliprdrBackend`] implementation), so
//! it cannot detect this on its own; [`LoopDetector`] is a building block
//! an embedder can drive from both sides of the bridge to break the cycle.
//!
//! # How it works
//!
//! 1. **Format hashing**: hashes the list of formats/content offered.
//! 2. **Content hashing**: optionally hashes actual clipboard bytes for
//!    deduplication independent of format list shape.
//! 3. **Time windowing**: only treats a match as a loop within a
//!    configurable recent window, so an unrelated later copy of the same
//!    content is not suppressed.
//! 4. **Source tracking**: only a match against the *opposite* source
//!    counts as a loop; two remote copies in a row are not one.
//! 5. **Rate limiting**: an optional per-source throttle as a
//!    belt-and-suspenders guard against rapid update storms even when
//!    hash correlation alone would not catch them.
//!
//! # Clock
//!
//! Every method that needs the current time takes an explicit `now_ms`
//! rather than reading a clock itself, matching
//! [`CliprdrBackend::now_ms()`](crate::backend::CliprdrBackend::now_ms).
//! This keeps the type usable on targets with no monotonic clock of their
//! own (e.g. `wasm32-unknown-unknown`) and makes tests deterministic.
//!
//! # Example
//!
//! ```
//! use ironrdp_cliprdr::loop_detector::{ClipboardSource, LoopDetector};
//! use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId};
//!
//! let mut detector = LoopDetector::new();
//! let formats = vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];
//!
//! // Record that the remote side just offered these formats.
//! detector.record_formats(&formats, ClipboardSource::Remote, 0);
//!
//! // Before syncing the same formats back out as a local change, check first.
//! if detector.would_cause_loop(&formats, ClipboardSource::Local, 10) {
//!     // Skip: this would just echo what the remote side already sent.
//! }
//! ```

use core::hash::{Hash as _, Hasher as _};
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;

use crate::pdu::ClipboardFormat;

/// Configuration for [`LoopDetector`].
#[derive(Debug, Clone)]
pub struct LoopDetectionConfig {
    /// Time window in milliseconds for detecting loops.
    pub window_ms: u64,

    /// Maximum number of operations to retain per history (format and content are tracked
    /// separately).
    pub max_history: usize,

    /// Whether to hash and track clipboard content, not just format lists.
    pub enable_content_hashing: bool,

    /// Optional rate limit in milliseconds.
    ///
    /// When set, sync operations are throttled to at most one per `rate_limit_ms` per source.
    /// This is a belt-and-suspenders guard against rapid clipboard updates even when hash
    /// correlation alone passes.
    pub rate_limit_ms: Option<u64>,
}

impl Default for LoopDetectionConfig {
    fn default() -> Self {
        Self {
            window_ms: 500,
            max_history: 10,
            enable_content_hashing: true,
            rate_limit_ms: None,
        }
    }
}

impl LoopDetectionConfig {
    /// Config with rate limiting enabled, other fields at their default.
    pub fn with_rate_limit(rate_limit_ms: u64) -> Self {
        Self {
            rate_limit_ms: Some(rate_limit_ms),
            ..Self::default()
        }
    }
}

/// Which side of the bridge a clipboard operation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardSource {
    /// The operation came from the CLIPRDR (RDP) side.
    Remote,
    /// The operation came from the local, non-RDP side the embedder bridges to.
    Local,
}

impl ClipboardSource {
    /// The other side.
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Self::Remote => Self::Local,
            Self::Local => Self::Remote,
        }
    }
}

#[derive(Debug, Clone)]
struct ClipboardOperation {
    hash: u64,
    source: ClipboardSource,
    recorded_at_ms: u64,
}

/// Detects and helps prevent clipboard synchronization loops.
///
/// See the [module documentation](self) for the detection strategy and calling convention.
#[derive(Debug)]
pub struct LoopDetector {
    config: LoopDetectionConfig,
    format_history: VecDeque<ClipboardOperation>,
    content_history: VecDeque<ClipboardOperation>,
    last_sync_remote_ms: Option<u64>,
    last_sync_local_ms: Option<u64>,
}

impl Default for LoopDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopDetector {
    /// Create a detector with the default configuration.
    pub fn new() -> Self {
        Self::with_config(LoopDetectionConfig::default())
    }

    /// Create a detector with a custom configuration.
    pub fn with_config(config: LoopDetectionConfig) -> Self {
        Self {
            config,
            format_history: VecDeque::new(),
            content_history: VecDeque::new(),
            last_sync_remote_ms: None,
            last_sync_local_ms: None,
        }
    }

    /// Record a format list operation from `source`.
    pub fn record_formats(&mut self, formats: &[ClipboardFormat], source: ClipboardSource, now_ms: u64) {
        let hash = Self::hash_formats(formats);
        self.format_history.push_back(ClipboardOperation {
            hash,
            source,
            recorded_at_ms: now_ms,
        });
        self.cleanup_history(now_ms);
    }

    /// Record content data from `source` for deduplication.
    ///
    /// No-op if `enable_content_hashing` is `false` in the active config.
    pub fn record_content(&mut self, data: &[u8], source: ClipboardSource, now_ms: u64) {
        if !self.config.enable_content_hashing {
            return;
        }

        let hash = Self::hash_content(data);
        self.content_history.push_back(ClipboardOperation {
            hash,
            source,
            recorded_at_ms: now_ms,
        });
        self.cleanup_history(now_ms);
    }

    /// Whether syncing `formats` out as `source` would just echo a recent operation from the
    /// opposite source.
    pub fn would_cause_loop(&self, formats: &[ClipboardFormat], source: ClipboardSource, now_ms: u64) -> bool {
        let hash = Self::hash_formats(formats);
        self.check_hash_collision(&self.format_history, hash, source, now_ms)
    }

    /// Whether syncing `data` out as `source` would just echo a recent operation from the
    /// opposite source. Always `false` if `enable_content_hashing` is disabled.
    pub fn would_cause_content_loop(&self, data: &[u8], source: ClipboardSource, now_ms: u64) -> bool {
        if !self.config.enable_content_hashing {
            return false;
        }

        let hash = Self::hash_content(data);
        self.check_hash_collision(&self.content_history, hash, source, now_ms)
    }

    /// Hash arbitrary data the same way [`Self::record_content`]/[`Self::would_cause_content_loop`]
    /// do internally, for a caller that wants to compute a hash once and compare it itself.
    pub fn compute_hash(data: &[u8]) -> u64 {
        Self::hash_content(data)
    }

    /// Clear all recorded history and rate-limit state.
    pub fn clear(&mut self) {
        self.format_history.clear();
        self.content_history.clear();
        self.last_sync_remote_ms = None;
        self.last_sync_local_ms = None;
    }

    /// Whether a sync as `source` is currently rate-limited.
    ///
    /// Always `false` unless `rate_limit_ms` is configured.
    pub fn is_rate_limited(&self, source: ClipboardSource, now_ms: u64) -> bool {
        let Some(rate_limit_ms) = self.config.rate_limit_ms else {
            return false;
        };

        let last_sync_ms = match source {
            ClipboardSource::Remote => self.last_sync_remote_ms,
            ClipboardSource::Local => self.last_sync_local_ms,
        };

        let Some(last_sync_ms) = last_sync_ms else {
            return false;
        };

        now_ms.saturating_sub(last_sync_ms) < rate_limit_ms
    }

    /// Record that a sync as `source` was performed, for future [`Self::is_rate_limited`] checks.
    pub fn record_sync(&mut self, source: ClipboardSource, now_ms: u64) {
        match source {
            ClipboardSource::Remote => self.last_sync_remote_ms = Some(now_ms),
            ClipboardSource::Local => self.last_sync_local_ms = Some(now_ms),
        }
    }

    /// Combined check: skip if either rate-limited or would cause a loop.
    pub fn should_skip_sync(&self, formats: &[ClipboardFormat], source: ClipboardSource, now_ms: u64) -> bool {
        if self.is_rate_limited(source, now_ms) {
            return true;
        }

        let hash = Self::hash_formats(formats);
        self.check_hash_collision(&self.format_history, hash, source, now_ms)
    }

    fn check_hash_collision(
        &self,
        history: &VecDeque<ClipboardOperation>,
        hash: u64,
        current_source: ClipboardSource,
        now_ms: u64,
    ) -> bool {
        let opposite = current_source.opposite();

        history
            .iter()
            .rev()
            .take_while(|op| now_ms.saturating_sub(op.recorded_at_ms) <= self.config.window_ms)
            .any(|op| op.source == opposite && op.hash == hash)
    }

    fn cleanup_history(&mut self, now_ms: u64) {
        let retain_window_ms = self.config.window_ms.saturating_mul(2);

        Self::cleanup_one(
            &mut self.format_history,
            retain_window_ms,
            self.config.max_history,
            now_ms,
        );
        Self::cleanup_one(
            &mut self.content_history,
            retain_window_ms,
            self.config.max_history,
            now_ms,
        );
    }

    fn cleanup_one(history: &mut VecDeque<ClipboardOperation>, retain_window_ms: u64, max_history: usize, now_ms: u64) {
        while let Some(front) = history.front() {
            if now_ms.saturating_sub(front.recorded_at_ms) > retain_window_ms {
                history.pop_front();
            } else {
                break;
            }
        }

        while history.len() > max_history {
            history.pop_front();
        }
    }

    fn hash_formats(formats: &[ClipboardFormat]) -> u64 {
        let mut hasher = DefaultHasher::new();
        for format in formats {
            format.id().0.hash(&mut hasher);
            if let Some(name) = format.name() {
                name.value().hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn hash_content(data: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        hasher.finish()
    }
}
