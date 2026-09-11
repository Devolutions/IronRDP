#![allow(clippy::print_stdout)]
#![allow(
    unused_crate_dependencies,
    reason = "the command-line binary owns these dependencies"
)]

use std::path::{Path, PathBuf};

pub mod bench;
pub mod capture;

pub fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest directory has no parent")
        .to_path_buf()
}
