#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    ironrdp_cef_server::run()
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("run the packaged ironrdp_cef_server.exe; Windows CEF requires the sandbox bootstrap")
}
