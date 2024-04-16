use std::fs;
use std::path::Path;

use anyhow::Context as _;

#[cfg(target_os = "windows")]
const OUTPUT_LIB_NAME: &str = "ironrdp.dll";
#[cfg(not(target_os = "windows"))]
const OUTPUT_LIB_NAME: &str = "libironrdp.so";

#[cfg(target_os = "windows")]
const DOTNET_NATIVE_LIB_NAME: &str = "DevolutionsIronRdp.dll";
#[cfg(not(target_os = "windows"))]
const DOTNET_NATIVE_LIB_NAME: &str = "libDevolutionsIronRdp.so";

pub(crate) fn build_dll(sh: &xshell::Shell, release: bool) -> anyhow::Result<()> {
    println!("Build IronRDP DLL");

    let mut args = vec!["build", "--package", "ffi"];
    if release {
        args.push("--release");
    }
    sh.cmd("cargo").args(&args).run()?;

    let profile_dir = if release { "release" } else { "debug" };

    let mut output_dir = sh.current_dir();
    output_dir.push("target");
    output_dir.push(profile_dir);

    let output_lib_path = output_dir.join(OUTPUT_LIB_NAME);
    let dotnet_native_lib_path = output_dir.join(DOTNET_NATIVE_LIB_NAME);

    // Copy build artifact to the proper native lib folder.
    // FIXME(@CBenoit): destination should follow the standard structure (e.g.: dependencies/runtimes/win-x64/native/DevolutionsPicky.dll)
    // See Picky FFI justfile for reference.
    std::fs::copy(&output_lib_path, &dotnet_native_lib_path).with_context(|| {
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

pub(crate) fn build_bindings(sh: &xshell::Shell, skip_dotnet_build: bool) -> anyhow::Result<()> {
    let dotnet_generated_path = "./dotnet/Devolutions.IronRdp/Generated/";
    let diplomat_config = "./dotnet-interop-conf.toml";

    // Check if diplomat tool is installed
    sh.change_dir("./ffi");
    let cwd = sh.current_dir();
    let generated_code_dir = cwd.join(dotnet_generated_path);
    if !generated_code_dir.exists() {
        anyhow::bail!("The directory {:?} does not exist", generated_code_dir);
    }
    remove_cs_files(&generated_code_dir)?;

    sh.cmd("diplomat-tool")
        .arg("dotnet")
        .arg(dotnet_generated_path)
        .arg("-l")
        .arg(diplomat_config)
        .run()?;

    if skip_dotnet_build {
        return Ok(());
    }

    sh.change_dir("./dotnet/Devolutions.IronRdp/");

    sh.cmd("dotnet").arg("build").run()?;

    Ok(())
}

/// Removes all `.cs` files in the given directory.
fn remove_cs_files(dir: &Path) -> anyhow::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("cs") {
                println!("Removing file: {:?}", path);
                fs::remove_file(path)?;
            }
        }
    }

    Ok(())
}
