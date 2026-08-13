#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
// Windows WebAuthn FFI call sites are dense; safety is documented at helper boundaries.
#![cfg_attr(windows, allow(clippy::undocumented_unsafe_blocks))]

//! Windows WebAuthn backend for [`ironrdp_rdpewa`].

#[cfg(windows)]
mod backend;
#[cfg(windows)]
mod cbor_map;
#[cfg(windows)]
mod ctap;

#[cfg(windows)]
pub use backend::{WindowsRdpewaBackend, WindowsRdpewaSession};
