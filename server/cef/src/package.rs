use std::path::PathBuf;
use std::process::Command;

use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"))?;
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts/bundle"));
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    let features = if cfg!(feature = "device-simulator") {
        "runtime,device-simulator"
    } else {
        "runtime"
    };
    command.args(["build", "--locked", "--features", features]);
    #[cfg(windows)]
    command.arg("--lib");
    #[cfg(not(windows))]
    command.args(["--bin", "ironrdp-cef-server", "--bin", "ironrdp-cef-helper"]);
    let status = command.status().context("failed to invoke cargo")?;
    anyhow::ensure!(status.success(), "native executable build failed");
    let target = cef::build_util::metadata::get_cargo_metadata()?
        .target_directory()
        .join("debug");
    #[cfg(target_os = "macos")]
    let path = bundle_mac(&output, &target)?;
    #[cfg(target_os = "linux")]
    let path = {
        let path = cef::build_util::linux::bundle(&output, &target, "ironrdp-cef-server")?;
        std::fs::copy(target.join("ironrdp-cef-helper"), output.join("ironrdp-cef-helper"))?;
        path
    };
    #[cfg(windows)]
    let path = {
        let path = cef::build_util::win::bundle(&output, &target, "ironrdp_cef_server")?;
        let cef = cef::sys::get_cef_dir().context("CEF distribution directory is unavailable")?;
        // Keep a console so Ctrl-C can request profile cleanup.
        std::fs::copy(cef.join("bootstrapc.exe"), &path)?;
        path
    };
    println!("{}", path.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn bundle_mac(output: &std::path::Path, target: &std::path::Path) -> anyhow::Result<PathBuf> {
    let info = cef::build_util::mac::BundleInfo::new(
        "ironrdp-cef-server",
        "com.devolutions.ironrdp.cef",
        "IronRDP Browser",
        "English",
        env!("CARGO_PKG_VERSION").parse()?,
    );
    let path = cef::build_util::mac::bundle(output, target, "ironrdp-cef-server", "ironrdp-cef-helper", None, info)?;
    let status = Command::new("plutil")
        .args(["-insert", "LSBackgroundOnly", "-bool", "YES"])
        .arg(path.join("Contents/Info.plist"))
        .status()
        .context("failed to invoke plutil")?;
    anyhow::ensure!(status.success(), "failed to mark the application as background-only");
    Ok(path)
}
