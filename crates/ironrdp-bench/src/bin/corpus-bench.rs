#![allow(unused_crate_dependencies)] // Dependencies are used by the library target.

const USAGE: &str = "usage: corpus-bench fetch
       corpus-bench list";

fn main() -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    match std::env::args().nth(1).as_deref() {
        Some("fetch") => ironrdp_bench::corpus::fetch(&mut out),
        Some("list") => ironrdp_bench::corpus::list(&mut out),
        _ => anyhow::bail!(USAGE),
    }
}
