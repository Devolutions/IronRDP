use std::fs;

use crate::prelude::*;

const IRON_REMOTE_DESKTOP_PATH: &str = "./web-client/iron-remote-desktop";
const IRON_REMOTE_DESKTOP_RDP_PATH: &str = "./web-client/iron-remote-desktop-rdp";
const IRON_SVELTE_CLIENT_PATH: &str = "./web-client/iron-svelte-client";
const IRON_REPLAY_PLAYER_PATH: &str = "./web-client/iron-replay-player";
const IRON_REPLAY_PLAYER_WASM_PATH: &str = "./web-client/iron-replay-player-wasm";
const IRON_SVELTE_REPLAY_CLIENT_PATH: &str = "./web-client/iron-svelte-replay-client";
const IRONRDP_WEB_PATH: &str = "./crates/ironrdp-web";
const IRONRDP_WEB_PACKAGE_JS_PATH: &str = "./crates/ironrdp-web/pkg/ironrdp_web.js";
const IRONRDP_WEB_REPLAY_PATH: &str = "./crates/ironrdp-web-replay";
const IRONRDP_WEB_REPLAY_PACKAGE_JS_PATH: &str = "./crates/ironrdp-web-replay/pkg/ironrdp_web_replay.js";

#[cfg(not(target_os = "windows"))]
const NPM: &str = "npm";
#[cfg(target_os = "windows")]
const NPM: &str = "npm.cmd";

/// Patch wasm-pack's generated JS to use a Vite-compatible import for the `.wasm` file.
///
/// wasm-pack emits `new URL('<name>_bg.wasm', import.meta.url)` which Vite cannot
/// resolve at build time. This replaces it with a static `import ... from '...'`
/// so the bundler can handle the asset.
fn patch_vite_wasm_url(js_path: &std::path::Path, wasm_filename: &str) -> anyhow::Result<()> {
    let content = fs::read_to_string(js_path)?;
    let import_line = format!("import wasmUrl from './{wasm_filename}?url';");
    let content = if content.contains(&import_line) {
        content
    } else {
        format!("{import_line}\n\n{content}")
    };
    let content = content.replace(&format!("new URL('{wasm_filename}', import.meta.url)"), "wasmUrl");
    fs::write(js_path, content)?;
    Ok(())
}

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
    run_cmd_in!(sh, IRON_REMOTE_DESKTOP_PATH, "{NPM} run test")?;
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
        let _env_guard = sh.push_env(
            "RUSTFLAGS",
            "-Ctarget-feature=+simd128,+bulk-memory --cfg getrandom_backend=\"wasm_js\"",
        );
        run_cmd_in!(sh, IRONRDP_WEB_PATH, "wasm-pack build --target web")?;
    }

    let ironrdp_web_js_file_path = sh.current_dir().join(IRONRDP_WEB_PACKAGE_JS_PATH);
    patch_vite_wasm_url(&ironrdp_web_js_file_path, "ironrdp_web_bg.wasm")?;

    run_cmd_in!(sh, IRON_SVELTE_CLIENT_PATH, "{NPM} run build-no-wasm")?;

    Ok(())
}

pub fn build_replay(sh: &Shell, wasm_pack_dev: bool) -> anyhow::Result<()> {
    let _s = Section::new("WEB-BUILD-REPLAY");

    if wasm_pack_dev {
        run_cmd_in!(sh, IRONRDP_WEB_REPLAY_PATH, "wasm-pack build --dev --target web")?;
    } else {
        let _env_guard = sh.push_env(
            "RUSTFLAGS",
            "-Ctarget-feature=+simd128,+bulk-memory --cfg getrandom_backend=\"wasm_js\"",
        );
        run_cmd_in!(sh, IRONRDP_WEB_REPLAY_PATH, "wasm-pack build --target web")?;
    }

    let js_path = sh.current_dir().join(IRONRDP_WEB_REPLAY_PACKAGE_JS_PATH);
    patch_vite_wasm_url(&js_path, "ironrdp_web_replay_bg.wasm")?;

    // Build the WASM adapter library (build-alone: WASM already patched above, avoids pre-build.js recursion).
    run_cmd_in!(sh, IRON_REPLAY_PLAYER_WASM_PATH, "{NPM} run build-alone")?;

    // Build the UI component library.
    run_cmd_in!(sh, IRON_REPLAY_PLAYER_PATH, "{NPM} run build")?;

    // Build the demo app (build-no-wasm: libs already built above)
    run_cmd_in!(sh, IRON_SVELTE_REPLAY_CLIENT_PATH, "{NPM} run build-no-wasm")?;

    Ok(())
}

pub fn install_replay(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-INSTALL-REPLAY");

    run_cmd_in!(sh, IRON_REPLAY_PLAYER_PATH, "{NPM} install")?;
    run_cmd_in!(sh, IRON_REPLAY_PLAYER_WASM_PATH, "{NPM} install")?;
    run_cmd_in!(sh, IRON_SVELTE_REPLAY_CLIENT_PATH, "{NPM} install")?;

    cargo_install(sh, &WASM_PACK)?;

    Ok(())
}

pub fn check_replay(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-CHECK-REPLAY");

    build_replay(sh, true)?;

    run_cmd_in!(sh, IRON_REPLAY_PLAYER_PATH, "{NPM} run check")?;
    run_cmd_in!(sh, IRON_REPLAY_PLAYER_PATH, "{NPM} run lint")?;
    run_cmd_in!(sh, IRON_REPLAY_PLAYER_PATH, "{NPM} run test")?;
    run_cmd_in!(sh, IRON_REPLAY_PLAYER_WASM_PATH, "{NPM} run check")?;
    run_cmd_in!(sh, IRON_REPLAY_PLAYER_WASM_PATH, "{NPM} run lint")?;
    run_cmd_in!(sh, IRON_SVELTE_REPLAY_CLIENT_PATH, "{NPM} run check")?;
    run_cmd_in!(sh, IRON_SVELTE_REPLAY_CLIENT_PATH, "{NPM} run lint")?;

    Ok(())
}

pub fn run_replay(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-RUN-REPLAY");

    run_cmd_in!(sh, IRON_SVELTE_REPLAY_CLIENT_PATH, "{NPM} run dev-no-wasm")?;

    Ok(())
}
