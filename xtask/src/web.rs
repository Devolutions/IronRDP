use crate::prelude::*;

const IRON_REMOTE_GUI_PATH: &str = "./web-client/iron-remote-gui";
const IRON_SVELTE_CLIENT_PATH: &str = "./web-client/iron-svelte-client";
const IRONRDP_WEB_PATH: &str = "./crates/ironrdp-web";

#[cfg(not(target_os = "windows"))]
const NPM: &str = "npm";
#[cfg(target_os = "windows")]
const NPM: &str = "npm.cmd";

pub fn install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-INSTALL");

    run_cmd_in!(sh, IRON_REMOTE_GUI_PATH, "{NPM} install")?;
    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} install")?;

    if !is_installed(sh, "wasm-pack") {
        if cfg!(target_os = "windows") {
            let _guard = sh.push_dir(LOCAL_CARGO_ROOT);

            cmd!(sh, "{NPM} install wasm-pack@{WASM_PACK_VERSION}").run()?;

            sh.copy_file(
                "./node_modules/binary-install/node_modules/.bin/wasm-pack.exe",
                "./bin/wasm-pack.exe",
            )?;

            sh.remove_path("./node_modules")?;
            sh.remove_path("./package-lock.json")?;
            sh.remove_path("./package.json")?;
        } else {
            // WORKAROUND: https://github.com/rustwasm/wasm-pack/issues/1203

            // NOTE: Install in debug because it's faster to compile and we don't need execution speed anyway.
            // NOTE: cargo-fuzz version is pinned so we don’t get different versions without intervention.
            cmd!(
                sh,
                "{CARGO} install
                --debug --locked
                --root {LOCAL_CARGO_ROOT}
                --no-default-features
                --features sys-openssl
                wasm-pack@{WASM_PACK_VERSION}"
            )
            .run()?;
        }
    }

    Ok(())
}

pub fn check(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-CHECK");

    build(sh, true)?;

    run_cmd_in!(sh, IRON_REMOTE_GUI_PATH, "{NPM} run check")?;
    run_cmd_in!(sh, IRON_REMOTE_GUI_PATH, "{NPM} run lint")?;
    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} run check")?;
    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} run lint")?;

    Ok(())
}

pub fn run(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-RUN");

    build(sh, false)?;

    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} run dev-no-wasm")?;

    Ok(())
}

fn build(sh: &Shell, wasm_pack_dev: bool) -> anyhow::Result<()> {
    if wasm_pack_dev {
        run_cmd_in!(sh, IRONRDP_WEB_PATH, "wasm-pack build --dev --target web")?;
    } else {
        run_cmd_in!(sh, IRONRDP_WEB_PATH, "wasm-pack build --target web")?;
    }

    run_cmd_in!(sh, IRON_REMOTE_GUI_PATH, "{NPM} run build-alone")?;
    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} run build-no-wasm")?;

    Ok(())
}
