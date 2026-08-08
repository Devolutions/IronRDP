#![allow(clippy::allow_attributes)]
#![cfg_attr(doc, doc = include_str!("../README.md"))]

#[cfg(windows)]
mod sandbox;
#[cfg(windows)]
mod windows_udk;

#[cfg(windows)]
pub use sandbox::{NetworkInformation, NetworkingMode, RunningSandboxVm, SandboxRuntime, SandboxVm};
#[cfg(windows)]
pub type Result<T> = windows_core::Result<T>;
