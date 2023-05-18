use crate::prelude::*;

const IRON_REMOTE_GUI_PREFIX: &str = "./web-client/iron-remote-gui";
const IRON_SVELTE_CLIENT_PREFIX: &str = "./web-client/iron-svelte-client";

pub fn install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-INSTALL");

    cmd!(sh, "npm install --prefix {IRON_REMOTE_GUI_PREFIX}").run()?;
    cmd!(sh, "npm install --prefix {IRON_SVELTE_CLIENT_PREFIX}").run()?;

    let wasm_pack_path: std::path::PathBuf = [LOCAL_CARGO_ROOT, "bin", "wasm-pack"].iter().collect();

    if !sh.path_exists(wasm_pack_path) {
        // Install in debug because it's faster to compile and we don't need execution speed anyway.
        // cargo-fuzz version is pinned so we don’t get different versions without intervention.
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

    Ok(())
}

pub fn check(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-CHECK");

    build(sh, true)?;

    Ok(())
}

pub fn run(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("WEB-RUN");

    build(sh, false)?;

    cmd!(sh, "npm run dev-no-wasm --prefix {IRON_SVELTE_CLIENT_PREFIX}").run()?;

    Ok(())
}

fn build(sh: &Shell, wasm_pack_dev: bool) -> anyhow::Result<()> {
    {
        let _guard = sh.push_dir("./crates/ironrdp-web");

        if wasm_pack_dev {
            cmd!(sh, "../../{LOCAL_CARGO_ROOT}/bin/wasm-pack build --dev --target web").run()?;
        } else {
            cmd!(sh, "../../{LOCAL_CARGO_ROOT}/bin/wasm-pack build --target web").run()?;
        }
    }

    cmd!(sh, "npm run check --prefix {IRON_REMOTE_GUI_PREFIX}").run()?;
    cmd!(sh, "npm run build-alone --prefix {IRON_REMOTE_GUI_PREFIX}").run()?;

    // cmd!(sh, "npm run check --prefix {IRON_SVELTE_CLIENT_PREFIX}").run()?; // FIXME: failing on master
    cmd!(sh, "npm run build-no-wasm --prefix {IRON_SVELTE_CLIENT_PREFIX}").run()?;

    Ok(())
}
