pub use anyhow::Context as _;
pub use xshell::{cmd, Shell};

pub use crate::bin_version::*;
pub use crate::section::Section;

pub const CARGO: &str = env!("CARGO");
pub const LOCAL_CARGO_ROOT: &str = ".cargo/local_root/";

pub const WASM_PACKAGES: &[&str] = &["ironrdp-web"];

pub const FUZZ_TARGETS: &[&str] = &["pdu_decoding", "rle_decompression", "bitmap_stream"];
