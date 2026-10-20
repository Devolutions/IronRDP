use std::fs::{self, create_dir_all};
use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::bin_install::{CargoPackageSource, is_pin_satisfied};
use crate::local_bin;
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

pub(crate) fn install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FFI-INSTALL");

    ensure_pin_matches_ffi_crate(sh)?;
    cargo_install(sh, &DIPLOMAT_TOOL)?;

    Ok(())
}

fn ensure_pin_matches_ffi_crate(sh: &Shell) -> anyhow::Result<()> {
    let CargoPackageSource::GitTag { url, tag, .. } = DIPLOMAT_TOOL.source else {
        anyhow::bail!("diplomat-tool must be pinned to a git tag");
    };

    let manifest: toml::Table = sh
        .read_file("ffi/Cargo.toml")
        .context("read ffi/Cargo.toml")?
        .parse()
        .context("parse ffi/Cargo.toml")?;
    let dependencies = manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .context("ffi/Cargo.toml has no dependencies table")?;

    for dep in ["diplomat", "diplomat-runtime"] {
        let dependency = dependencies.get(dep).and_then(toml::Value::as_table);
        anyhow::ensure!(
            dependency.and_then(|d| d.get("git")).and_then(toml::Value::as_str) == Some(url)
                && dependency.and_then(|d| d.get("tag")).and_then(toml::Value::as_str) == Some(tag),
            "ffi/Cargo.toml `{dep}` must use git `{url}` at tag `{tag}`, like diplomat-tool in xtask/src/bin_version.rs"
        );
    }

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

    ensure_pin_matches_ffi_crate(sh)?;

    if !is_installed(sh, &DIPLOMAT_TOOL) || !is_pin_satisfied(sh, &DIPLOMAT_TOOL) {
        anyhow::bail!("`diplomat-tool` is missing or not the pinned version, run `cargo xtask ffi install`");
    }

    // Invoke by absolute path so a stray `diplomat-tool` on PATH cannot shadow the pinned local tool.
    let diplomat_tool = sh
        .current_dir()
        .join(local_bin())
        .join(DIPLOMAT_TOOL.binary_name)
        .with_extension(std::env::consts::EXE_EXTENSION);

    let dotnet_generated_path = "./dotnet/Devolutions.IronRdp/Generated/";
    let diplomat_config = "./dotnet-interop-conf.toml";

    sh.change_dir("./ffi");
    let cwd = sh.current_dir();
    let generated_code_dir = cwd.join(dotnet_generated_path);
    if !generated_code_dir.exists() {
        anyhow::bail!("The directory {} does not exist", generated_code_dir.display());
    }
    remove_cs_files(&generated_code_dir)?;

    sh.cmd(diplomat_tool)
        .arg("dotnet")
        .arg(dotnet_generated_path)
        .arg("-c")
        .arg(diplomat_config)
        .run()?;

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
