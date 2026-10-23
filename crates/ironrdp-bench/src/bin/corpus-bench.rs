#![allow(unused_crate_dependencies)] // Dependencies are used by the library target.

const USAGE: &str = "usage: corpus-bench fetch
       corpus-bench list
       corpus-bench check [--capture <ID>]";

fn main() -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["fetch"] => ironrdp_bench::corpus::fetch(&mut out),
        ["list"] => ironrdp_bench::corpus::list(&mut out),
        ["check"] => ironrdp_bench::corpus::check(None, &mut out),
        ["check", "--capture", id] => ironrdp_bench::corpus::check(Some(id), &mut out),
        _ => anyhow::bail!(USAGE),
    }
}
