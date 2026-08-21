#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary
#![allow(clippy::panic, reason = "panic is fine in tests")]
#![allow(clippy::std_instead_of_core, reason = "std is fine in integration tests")]
#![allow(clippy::unwrap_used, reason = "unwrap is fine in tests")]

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(feature = "full")]
mod agent;
mod async_framed;
mod capture_helpers;
mod capture_replay;
#[cfg(feature = "full")]
mod client_config;
mod dvc_pipe_proxy;
#[cfg(feature = "rustls")]
mod e2e;
mod gateway_detect;
mod mstsgu;
mod rdpeudp_tokio;
mod vmconnect;
mod volume;

fn workspace_binary(package: &str, binary: &str) -> PathBuf {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testsuite must be in the workspace")
        .to_owned();
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root)
        .args(["build", "--quiet", "--package", package, "--bin", binary])
        .status()
        .expect("build binary");
    assert!(status.success(), "build {binary} binary");

    let mut path = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace_root.join(path)
            }
        })
        .unwrap_or_else(|| workspace_root.join("target"));
    if let Some(target) = std::env::var_os("CARGO_BUILD_TARGET") {
        path.push(target);
    }
    path.push("debug");
    path.push(format!("{binary}{}", std::env::consts::EXE_SUFFIX));
    assert!(path.is_file(), "{binary} binary does not exist: {}", path.display());
    path
}
