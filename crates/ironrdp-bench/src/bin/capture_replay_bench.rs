#![expect(
    clippy::print_stderr,
    reason = "the command reports qualified workload failures to its caller"
)]
#![expect(
    clippy::print_stdout,
    reason = "the command reports payload-free replay measurements to its caller"
)]
#![allow(unused_crate_dependencies)] // The package also contains unrelated benchmark dependencies.

use core::str::FromStr as _;
use std::process::ExitCode;

use ironrdp_bench::replay::{PartialReplayId, PartialReplayWorkload};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("capture replay benchmark failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let Some(argument) = arguments.next() else {
        return Err(usage().to_owned());
    };
    if argument == "--help" || argument == "-h" {
        println!("{}", usage());
        return Ok(());
    }
    if argument != "--capture" {
        return Err(usage().to_owned());
    }
    let selector = arguments.next().ok_or_else(|| usage().to_owned())?;
    if arguments.next().is_some() {
        return Err(usage().to_owned());
    }

    let id = PartialReplayId::from_str(&selector).map_err(|error| error.to_string())?;
    let workload = PartialReplayWorkload::prepare(id).map_err(|error| error.to_string())?;
    let measurement = workload.verify().map_err(|error| error.to_string())?;
    println!(
        "workload=partial-replay/{}\trouted_pdus={}\tgraphics_updates={}",
        workload.id().as_str(),
        measurement.routed_pdus,
        measurement.graphics_updates
    );
    Ok(())
}

const fn usage() -> &'static str {
    "usage: capture-replay-bench --capture <no-nla-accepted|no-nla-smartcard>"
}
