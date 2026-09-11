// We pin the binaries to specific versions so we use the same artifact everywhere.
// Hash of this file is used in CI for caching.

use crate::bin_install::CargoPackage;

pub const CARGO_FUZZ: CargoPackage = CargoPackage::new("cargo-fuzz", "0.13.2");
pub const CARGO_HACK: CargoPackage = CargoPackage::new("cargo-hack", "0.6.45");
pub const WASM_PACK: CargoPackage = CargoPackage::new("wasm-pack", "0.15.0");
pub const TYPOS_CLI: CargoPackage = CargoPackage::new("typos-cli", "1.48.0").with_binary_name("typos");
// WIP pin matching ffi/Cargo.toml / workspace patch (diplomat#1250 branch).
// Install: cargo install --git https://github.com/irvingoujAtDevolution/diplomat.git \
//   --rev ab68f7a4bf1b910729d0e6ece4cb7da31e616306 diplomat-tool
// crates.io 0.7.1 cannot generate against this pin.
pub const DIPLOMAT_TOOL: CargoPackage = CargoPackage::new("diplomat-tool", "0.16.0");

pub const WABT_VERSION: &str = "1.0.41";
pub const NIGHTLY_TOOLCHAIN: &str = "nightly-2026-03-05";
