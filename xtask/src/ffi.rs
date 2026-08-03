use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::prelude::*;

#[cfg(target_os = "windows")]
const OUTPUT_LIB_NAME: &str = "ironrdp.dll";
#[cfg(target_os = "linux")]
const OUTPUT_LIB_NAME: &str = "libironrdp.so";
#[cfg(target_os = "macos")]
const OUTPUT_LIB_NAME: &str = "libironrdp.dylib";

#[cfg(target_os = "windows")]
const DOTNET_NATIVE_LIB_NAME: &str = "DevolutionsIronRdp.dll";
#[cfg(target_os = "linux")]
const DOTNET_NATIVE_LIB_NAME: &str = "libDevolutionsIronRdp.so";
#[cfg(target_os = "macos")]
const DOTNET_NATIVE_LIB_NAME: &str = "libDevolutionsIronRdp.dylib";

#[cfg(target_os = "windows")]
const DOTNET_NATIVE_LIB_PATH: &str = "dependencies/runtimes/win-x64/native/";
#[cfg(target_os = "linux")]
const DOTNET_NATIVE_LIB_PATH: &str = "dependencies/runtimes/linux-x64/native/";
#[cfg(target_os = "macos")]
const DOTNET_NATIVE_LIB_PATH: &str = "dependencies/runtimes/osx-x64/native/";

const DIPLOMAT_GIT_URL: &str = "https://github.com/irvingoujAtDevolution/diplomat.git";
// Keep in sync with the `diplomat` / `diplomat-runtime` rev pinned in ffi/Cargo.toml —
// `diplomat-tool` is the codegen half of the same monorepo and must match the
// runtime crates commit-for-commit, or generated bindings drift from what the
// runtime types actually support.
const DIPLOMAT_REV: &str = "822b0b3effd892ca5babae7c1872d8c6be0685de";

pub(crate) fn install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FFI-INSTALL");

    install_diplomat_tool(sh)?;

    Ok(())
}

fn install_diplomat_tool(sh: &Shell) -> anyhow::Result<()> {
    if is_installed(sh, "diplomat-tool") {
        trace!("Refresh diplomat-tool from {DIPLOMAT_GIT_URL}@{DIPLOMAT_REV}");
    } else {
        trace!("Install diplomat-tool from {DIPLOMAT_GIT_URL}@{DIPLOMAT_REV}");
    }

    // This pinned commit's `diplomat_core` `hir` feature uses `if let` guards,
    // which our workspace's pinned 1.89.0 toolchain (rust-toolchain.toml) doesn't
    // support. `diplomat-tool` is a standalone dev-time codegen binary, not linked
    // into `ffi`, so it doesn't need to match that pin — build it with `rustup`'s
    // default stable toolchain instead of the directory-overridden one.
    sh.cmd("rustup")
        .arg("run")
        .arg("stable")
        .arg("cargo")
        .arg("install")
        .arg("--debug")
        .arg("--locked")
        .arg("--force")
        .arg("--root")
        .arg(crate::LOCAL_CARGO_ROOT)
        .arg("--git")
        .arg(DIPLOMAT_GIT_URL)
        .arg("--rev")
        .arg(DIPLOMAT_REV)
        .arg("diplomat-tool")
        .run()?;

    Ok(())
}

pub(crate) fn build_dynamic_lib(sh: &Shell, release: bool) -> anyhow::Result<()> {
    let _s = Section::new("BUILD-DYNAMIC-LIBRARY");

    println!("Build IronRDP DLL");

    let mut args = vec!["build", "--package", "ffi"];
    if release {
        args.push("--release");
    }
    sh.cmd("cargo").args(&args).run()?;

    let profile_dir = if release { "release" } else { "debug" };

    let root_dir = sh.current_dir();
    let target_dir = root_dir.join("target");
    let profile_dir = target_dir.join(profile_dir);

    let output_lib_path = profile_dir.join(OUTPUT_LIB_NAME);

    let dotnet_native_lib_dir_path: PathBuf = DOTNET_NATIVE_LIB_PATH.parse()?;
    let dotnet_native_lib_path = root_dir.join(&dotnet_native_lib_dir_path).join(DOTNET_NATIVE_LIB_NAME);

    create_dir_all(&dotnet_native_lib_dir_path)
        .with_context(|| format!("failed to create directory {}", dotnet_native_lib_dir_path.display()))?;

    fs::copy(&output_lib_path, &dotnet_native_lib_path).with_context(|| {
        format!(
            "failed to copy {} to {}",
            output_lib_path.display(),
            dotnet_native_lib_path.display()
        )
    })?;

    println!(
        "Copied {} to {}",
        output_lib_path.display(),
        dotnet_native_lib_path.display()
    );

    Ok(())
}

pub(crate) fn build_bindings(sh: &Shell, skip_dotnet_build: bool) -> anyhow::Result<()> {
    let _s = Section::new("BUILD-BINDINGS");

    let root_dir = sh.current_dir();
    let local_diplomat_tool = root_dir.join("..").join("diplomat").join("tool").join("Cargo.toml");
    let use_local_diplomat_tool = local_diplomat_tool.exists();

    if !use_local_diplomat_tool && !is_installed(sh, "diplomat-tool") {
        anyhow::bail!("`diplomat-tool` binary is missing. Please run `cargo xtask ffi install`.");
    }

    let dotnet_generated_path = "./dotnet/Devolutions.IronRdp/Generated/";
    let diplomat_config = "./dotnet-interop-conf.toml";

    // Check if diplomat tool is installed
    sh.change_dir("./ffi");
    let cwd = sh.current_dir();
    let generated_code_dir = cwd.join(dotnet_generated_path);
    if !generated_code_dir.exists() {
        anyhow::bail!("The directory {} does not exist", generated_code_dir.display());
    }
    remove_cs_files(&generated_code_dir)?;

    if use_local_diplomat_tool {
        // Same toolchain mismatch as `install_diplomat_tool`: the pinned commit's
        // `diplomat_core` `hir` feature needs a newer Rust than our workspace's
        // 1.89.0 pin (rust-toolchain.toml), which would otherwise apply here since
        // cwd is inside the IronRDP tree.
        sh.cmd("rustup")
            .arg("run")
            .arg("stable")
            .arg("cargo")
            .arg("run")
            .arg("--manifest-path")
            .arg(local_diplomat_tool)
            .arg("--")
            .arg("dotnet")
            .arg(dotnet_generated_path)
            .arg("-c")
            .arg(diplomat_config)
            .run()?;
    } else {
        sh.cmd("diplomat-tool")
            .arg("dotnet")
            .arg(dotnet_generated_path)
            .arg("-c")
            .arg(diplomat_config)
            .run()?;
    }

    if skip_dotnet_build {
        return Ok(());
    }

    sh.change_dir("./dotnet/Devolutions.IronRdp/");

    cmd!(sh, "dotnet build").run()?;

    Ok(())
}

/// Removes all `.cs` files in the given directory.
fn remove_cs_files(dir: &Path) -> anyhow::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("cs") {
                println!("Removing file: {path:?}");
                fs::remove_file(path)?;
            }
        }
    }

    Ok(())
}
