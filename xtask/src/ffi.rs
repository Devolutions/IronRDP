pub(crate) fn build_dll(sh: &xshell::Shell, release: bool) -> anyhow::Result<()> {
    let mut args = vec!["build", "--package", "ffi"];
    if release {
        args.push("--release");
    }
    sh.cmd("cargo").args(&args).run()?;

    let target_dir = if release { "release" } else { "debug" };

    let mut path = sh.current_dir();
    path.push("target");
    path.push(target_dir);

    let dll_name = "ironrdp.dll";
    let devolution_dll_name = "DevolutionsIronRdp.dll";

    let mut dll_path = path.clone();
    dll_path.push(dll_name);

    let mut devolution_dll_path = path.clone();
    devolution_dll_path.push(devolution_dll_name);

    // copy dll_path to devolution_dll_path
    std::fs::copy(&dll_path, &devolution_dll_path)?;
    println!("Copied {:?} to {:?}", dll_path, devolution_dll_path);

    Ok(())
}

use std::fs;
use std::path::Path;

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

    sh.change_dir("./dotnet");
    sh.change_dir("./Devolutions.IronRdp");

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
