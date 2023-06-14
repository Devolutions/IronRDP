pub use anyhow::Context as _;
pub use xshell::{cmd, Shell};

pub use crate::bin_version::*;
pub use crate::section::Section;
pub use crate::{is_installed, is_verbose, list_files, local_bin, local_cargo_root, set_verbose};

pub const CARGO: &str = env!("CARGO");
#[cfg(target_os = "windows")]
pub const LOCAL_CARGO_ROOT: &str = ".cargo\\local_root\\";
#[cfg(not(target_os = "windows"))]
pub const LOCAL_CARGO_ROOT: &str = ".cargo/local_root/";

pub const WASM_PACKAGES: &[&str] = &["ironrdp-web"];

pub const FUZZ_TARGETS: &[&str] = &["pdu_decoding", "rle_decompression", "bitmap_stream"];
