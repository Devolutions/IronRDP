#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![warn(unsafe_op_in_unsafe_fn)]
#![warn(invalid_reference_casting)]
#![warn(clippy::undocumented_unsafe_blocks)]
#![warn(clippy::multiple_unsafe_ops_per_block)]
#![warn(clippy::transmute_ptr_to_ptr)]
#![warn(clippy::as_ptr_cast_mut)]
#![warn(clippy::cast_ptr_alignment)]
#![warn(clippy::fn_to_numeric_cast_any)]
#![warn(clippy::ptr_cast_constness)]

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use crate::windows::{HWND, WinClipboard, WinCliprdrError, WinCliprdrResult};

mod stub;
use std::sync::OnceLock;
use std::time::Instant;

pub use crate::stub::{StubClipboard, StubCliprdrBackend};

/// Process-wide monotonic clock epoch for `CliprdrBackend::now_ms()` on native platforms.
///
/// Uses a lazily-initialized `Instant` so that all backends in the same process
/// share the same zero-point, producing comparable timestamps.
fn epoch() -> &'static Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now)
}

/// Returns monotonic milliseconds since process start, for use by native
/// `CliprdrBackend` implementations.
pub fn native_now_ms() -> u64 {
    u64::try_from(epoch().elapsed().as_millis()).unwrap_or(u64::MAX)
}
