use std::fs;

use crate::prelude::*;

const IRON_REMOTE_DESKTOP_PATH: &str = "./web-client/iron-remote-desktop";
const IRON_REMOTE_DESKTOP_RDP_PATH: &str = "./web-client/iron-remote-desktop-rdp";
const IRON_SVELTE_CLIENT_PATH: &str = "./web-client/iron-svelte-client";
const IRONRDP_WEB_PATH: &str = "./crates/ironrdp-web";
const IRONRDP_WEB_PACKAGE_JS_PATH: &str = "./crates/ironrdp-web/pkg/ironrdp_web.js";

#[cfg(not(target_os = "windows"))]
const NPM: &str = "npm";
#[cfg(target_os = "windows")]
const NPM: &str = "npm.cmd";

pub fn install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-INSTALL");

    run_cmd_in!(sh, IRON_REMOTE_DESKTOP_PATH, "{NPM} install")?;
    run_cmd_in!(sh, IRON_REMOTE_DESKTOP_RDP_PATH, "{NPM} install")?;
    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} install")?;

    cargo_install(sh, &WASM_PACK)?;

    Ok(())
}

pub fn check(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-CHECK");

    build(sh, true)?;

    run_cmd_in!(sh, IRON_REMOTE_DESKTOP_PATH, "{NPM} run check")?;
    run_cmd_in!(sh, IRON_REMOTE_DESKTOP_PATH, "{NPM} run lint")?;
    run_cmd_in!(sh, IRON_REMOTE_DESKTOP_RDP_PATH, "{NPM} run check")?;
    run_cmd_in!(sh, IRON_REMOTE_DESKTOP_RDP_PATH, "{NPM} run lint")?;
    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} run check")?;
    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} run lint")?;

    Ok(())
}

pub fn run(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-RUN");

    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} run dev-no-wasm")?;

    Ok(())
}

pub fn build(sh: &Shell, wasm_pack_dev: bool) -> anyhow::Result<()> {
    if wasm_pack_dev {
        run_cmd_in!(sh, IRONRDP_WEB_PATH, "wasm-pack build --dev --target web")?;
    } else {
        let _env_guard = sh.push_env("RUSTFLAGS", "-Ctarget-feature=+simd128,+bulk-memory");
        run_cmd_in!(sh, IRONRDP_WEB_PATH, "wasm-pack build --target web")?;
    }

    let ironrdp_web_js_file_path = sh.current_dir().join(IRONRDP_WEB_PACKAGE_JS_PATH);

    let ironrdp_web_js_content = fs::read_to_string(&ironrdp_web_js_file_path)?;

    // Modify the js file to get rid of the `URL` object.
    // Vite doesn't work properly with inlined urls in `new URL(url, import.meta.url)`.
    let ironrdp_web_js_content = format!(
        "import wasmUrl from './ironrdp_web_bg.wasm?url';\n\n{}",
        ironrdp_web_js_content
    );
    let ironrdp_web_js_content =
        ironrdp_web_js_content.replace("new URL('ironrdp_web_bg.wasm', import.meta.url)", "wasmUrl");

    fs::write(&ironrdp_web_js_file_path, ironrdp_web_js_content)?;

    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} run build-no-wasm")?;

    Ok(())
}
