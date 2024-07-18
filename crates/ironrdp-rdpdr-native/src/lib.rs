#![doc = include_str!("../README.md")]
#![warn(unsafe_op_in_unsafe_fn)]
#![warn(invalid_reference_casting)]
#![warn(pointer_structural_match)]
#![warn(clippy::undocumented_unsafe_blocks)]
#![warn(clippy::multiple_unsafe_ops_per_block)]
#![warn(clippy::transmute_ptr_to_ptr)]
#![warn(clippy::as_ptr_cast_mut)]
#![warn(clippy::cast_ptr_alignment)]
#![warn(clippy::fn_to_numeric_cast_any)]
#![warn(clippy::ptr_cast_constness)]

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[macro_use]
extern crate tracing;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod nix;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use nix::simple;
