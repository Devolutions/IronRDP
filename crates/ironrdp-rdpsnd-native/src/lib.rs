#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
// `anyhow` and `tracing-subscriber` are dev-deps used only by the `cpal`
// example binary, but `unused_crate_dependencies` still flags them on the
// lib target. The `[lib] test = false` setting makes a `#[cfg(test)]`
// workaround dead code, so the suppression has to apply unconditionally.
#![allow(unused_crate_dependencies)]

pub mod cpal;
pub mod error;

pub use error::{RdpsndNativeError, RdpsndNativeErrorKind, RdpsndNativeResult};
