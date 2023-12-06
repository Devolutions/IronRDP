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

    cargo_install(sh, &WASM_PACK)?;

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
