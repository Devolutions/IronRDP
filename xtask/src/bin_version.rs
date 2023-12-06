// We pin the binaries to specific versions so we use the same artifact everywhere.
// Hash of this file is used in CI for caching.

use crate::bin_install::CargoPackage;

pub const CARGO_FUZZ: CargoPackage = CargoPackage::new("cargo-fuzz", "0.11.2");
pub const CARGO_LLVM_COV: CargoPackage = CargoPackage::new("cargo-llvm-cov", "0.5.37");
pub const GRCOV: CargoPackage = CargoPackage::new("grcov", "0.8.19");
pub const WASM_PACK: CargoPackage = CargoPackage::new("wasm-pack", "0.12.1");
pub const TYPOS_CLI: CargoPackage = CargoPackage::new("typos-cli", "1.16.23").with_binary_name("typos");

pub const WABT_VERSION: &str = "1.0.33";
