#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

pub mod ipc;
pub mod transport;
#[cfg(feature = "__test")]
pub mod wire;
#[cfg(not(feature = "__test"))]
#[expect(
    unreachable_pub,
    reason = "the __test feature exposes this module to the shared integration tests"
)]
pub(crate) mod wire;
