#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    // SAFETY: `main` runs single-threaded before Tokio, CEF, or any framework threads start.
    // Pinning `TZ=UTC` ensures ICU's host-timezone fallback in the browser and every child
    // renderer, OOPIF, and worker process is `UTC`, never the container host's local timezone.
    unsafe {
        std::env::set_var("TZ", "UTC");
        std::env::set_var("CEF_ISOLATE_TIMEZONE", "1");
    }
    ironrdp_cef_server::run()
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("run the packaged ironrdp_cef_server.exe; Windows CEF requires the sandbox bootstrap")
}
