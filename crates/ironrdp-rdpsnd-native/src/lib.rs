#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
// `anyhow` and `tracing-subscriber` are dev-deps used only by the `cpal`
// example binary, but `unused_crate_dependencies` still flags them on the
// lib target. The `[lib] test = false` setting makes a `#[cfg(test)]`
// workaround dead code, so the suppression has to apply unconditionally.
#![allow(unused_crate_dependencies)]

#[cfg(feature = "capture")]
pub mod capture;
pub mod cpal;
pub mod error;

#[cfg(feature = "capture")]
pub use capture::{RdpeaiCaptureBackend, is_pcm_capture_format, take_capture_packets};
pub use error::{RdpsndNativeError, RdpsndNativeErrorKind, RdpsndNativeResult};
