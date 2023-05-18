pub use anyhow::Context as _;
pub use xshell::{cmd, Shell};

pub use crate::section::Section;

pub const CARGO: &str = env!("CARGO");
pub const LOCAL_CARGO_ROOT: &str = ".cargo/local_root/";

pub const CARGO_FUZZ_VERSION: &str = "0.11.2";
pub const GRCOV_VERSION: &str = "0.8.18";
pub const WASM_PACK_VERSION: &str = "0.11.1";

pub const WASM_PACKAGES: &[&str] = &["ironrdp-web"];

pub const FUZZ_TARGETS: &[&str] = &["pdu_decoding", "rle_decompression", "bitmap_stream"];
