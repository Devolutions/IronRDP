// The binary uses only a subset of the library's dependencies; the rest are used by the lib target.
#![allow(unused_crate_dependencies)]

use clap::Parser as _;
use ironrdp_agent::cli::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let result = ironrdp_agent::cli::run(Cli::parse()).await;
    if let Err(error) = result {
        if let Some(exit_code) = ironrdp_agent::cli::remote_exit_code(&error) {
            std::process::exit(exit_code);
        }
        return Err(error);
    }
    Ok(())
}
