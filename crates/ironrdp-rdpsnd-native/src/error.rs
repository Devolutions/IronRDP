//! Typed error types for `ironrdp-rdpsnd-native`.

/// Categorises failures in `ironrdp-rdpsnd-native` operations.
///
/// Bug-shaped conditions are intentionally absent: misuse of this crate's
/// public API should panic or trip `debug_assert!`, not return `Err`.
#[derive(Debug)]
#[non_exhaustive]
pub enum RdpsndNativeErrorKind {
    /// Server requested an audio format outside the supported set (wave
    /// format, channel count, or bit depth).
    UnsupportedFormat,
    /// The Opus decoder failed to initialise. Source carries the underlying
    /// `opus2::Error` when available.
    OpusInit,
    /// No usable audio output device or no supported output configuration
    /// for the requested format. Source carries the underlying `cpal` error
    /// when available.
    AudioDevice,
    /// The `cpal` output stream could not be built. Source carries the
    /// underlying `cpal::BuildStreamError`.
    StreamBuild,
}

impl core::fmt::Display for RdpsndNativeErrorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedFormat => write!(f, "unsupported audio format"),
            Self::OpusInit => write!(f, "Opus decoder initialisation"),
            Self::AudioDevice => write!(f, "audio output device"),
            Self::StreamBuild => write!(f, "output audio stream build"),
        }
    }
}

impl core::error::Error for RdpsndNativeErrorKind {}

pub type RdpsndNativeError = ironrdp_error::Error<RdpsndNativeErrorKind>;
pub type RdpsndNativeResult<T> = Result<T, RdpsndNativeError>;
