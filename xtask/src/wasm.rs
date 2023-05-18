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

        let output = cmd!(sh, "wasm2wat ./target/wasm32-unknown-unknown/debug/{artifact_name}").output()?;
        let stdout = std::str::from_utf8(&output.stdout).context("wasm2wat output is not valid UTF-8")?;

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

    Ok(())
}
