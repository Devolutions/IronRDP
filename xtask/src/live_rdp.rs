use crate::prelude::*;

pub fn run(
    sh: &Shell,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password_env: Option<String>,
    domain: Option<String>,
    artifacts_dir: Option<String>,
) -> anyhow::Result<()> {
    let _s = Section::new("LIVE-RDP-RUN");

    cmd!(sh, "{CARGO} build --release -p ironrdp-agent -p ironrdp-agent-e2e").run()?;

    let bin_dir = sh.current_dir().join("target").join("release");
    let suffix = std::env::consts::EXE_SUFFIX;
    let agent_bin = bin_dir.join(format!("ironrdp-agent{suffix}"));
    let e2e_bin = bin_dir.join(format!("ironrdp-agent-e2e{suffix}"));

    let mut args: Vec<String> = vec!["--agent-bin".to_owned(), agent_bin.display().to_string()];
    if let Some(host) = host {
        args.extend(["--host".to_owned(), host]);
    }
    if let Some(port) = port {
        args.extend(["--port".to_owned(), port.to_string()]);
    }
    if let Some(username) = username {
        args.extend(["--username".to_owned(), username]);
    }
    if let Some(password_env) = password_env {
        args.extend(["--password-env".to_owned(), password_env]);
    }
    if let Some(domain) = domain {
        args.extend(["--domain".to_owned(), domain]);
    }
    if let Some(artifacts_dir) = artifacts_dir {
        args.extend(["--artifacts-dir".to_owned(), artifacts_dir]);
    }

    sh.cmd(&e2e_bin).args(&args).run()?;

    Ok(())
}
