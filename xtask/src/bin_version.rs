// We pin the binaries to specific versions so we use the same artifact everywhere.
// Hash of this file is used in CI for caching.

use crate::bin_install::CargoPackage;

pub const CARGO_FUZZ: CargoPackage = CargoPackage::new("cargo-fuzz", "0.12.0");
pub const CARGO_HACK: CargoPackage = CargoPackage::new("cargo-hack", "0.6.44");
pub const WASM_PACK: CargoPackage = CargoPackage::new("wasm-pack", "0.13.1");
pub const TYPOS_CLI: CargoPackage = CargoPackage::new("typos-cli", "1.29.5").with_binary_name("typos");

pub const WABT_VERSION: &str = "1.0.36";
pub const NIGHTLY_TOOLCHAIN: &str = "nightly-2026-03-05";
