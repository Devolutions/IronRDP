use crate::prelude::*;

pub fn check(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WASM-CHECK");

    for package in WASM_PACKAGES {
        println!("Check {package}");

        cmd!(
            sh,
            "{CARGO} rustc --locked --target wasm32-unknown-unknown --package {package} --lib --crate-type cdylib"
        )
        .run()?;

        // When building a library, `-` in the artifact name are replaced by `_`
        let artifact_name = format!("{}.wasm", package.replace('-', "_"));

        if is_verbose() {
            cmd!(sh, "wasm2wat --version").run()?;
            list_files(sh, "./target/")?;
            list_files(sh, "./target/wasm32-unknown-unknown/debug/")?;
            list_files(sh, local_bin())?;
        }

        let stdout = cmd!(sh, "wasm2wat ./target/wasm32-unknown-unknown/debug/{artifact_name}").read()?;

        if stdout.contains("import \"env\"") {
            anyhow::bail!("Found undefined symbols in generated wasm file");
        }
    }

    println!("All good!");

    Ok(())
}

pub fn install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WASM-INSTALL");

    cmd!(sh, "rustup target add wasm32-unknown-unknown").run()?;

    match cmd!(sh, "wasm2wat --version").read() {
        Ok(version) => println!("Found wasm2wat {version}"),
        Err(e) => {
            trace!("{e}");
            install_wasm2wat(sh)?;
        }
    }

    Ok(())
}

fn install_wasm2wat(sh: &Shell) -> anyhow::Result<()> {
    println!("Installing wasm2wat in local root...");

    let _guard = sh.push_dir(LOCAL_CARGO_ROOT);

    let platform_suffix = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos-12"
    } else {
        "ubuntu"
    };

    let url = format!(
        "https://github.com/WebAssembly/wabt/releases/download/{WABT_VERSION}/wabt-{WABT_VERSION}-{platform_suffix}.tar.gz"
    );

    cmd!(sh, "curl --location --remote-header-name {url} --output wabt.tar.gz").run()?;

    if is_verbose() {
        list_files(sh, ".")?;
    }

    sh.create_dir("wabt")?;
    cmd!(sh, "tar xf wabt.tar.gz -C ./wabt --strip-components 1").run()?;

    if is_verbose() {
        list_files(sh, "./wabt")?;
        list_files(sh, "./wabt/bin")?;
    }

    trace!("Copy wasm2wat to local bin");

    if cfg!(target_os = "windows") {
        sh.copy_file("./wabt/bin/wasm2wat.exe", "./bin/wasm2wat.exe")?;
    } else {
        sh.copy_file("./wabt/bin/wasm2wat", "./bin/wasm2wat")?;
    }

    trace!("Clean artifacts");

    sh.remove_path("./wabt.tar.gz")?;
    sh.remove_path("./wabt")?;

    if is_verbose() {
        list_files(sh, ".")?;
    }

    Ok(())
}
