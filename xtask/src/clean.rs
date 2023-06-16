use crate::prelude::*;

pub fn workspace(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("CLEAN");

    println!("Remove wasm package folder…");
    sh.remove_path("./crates/ironrdp-web/pkg")?;
    println!("Done.");

    println!("Remove npm folders…");
    sh.remove_path("./web-client/iron-remote-gui/node_modules")?;
    sh.remove_path("./web-client/iron-remote-gui/dist")?;
    sh.remove_path("./web-client/iron-svelte-client/node_modules")?;
    println!("Done.");

    cmd!(sh, "{CARGO} clean").run()?;

    Ok(())
}
