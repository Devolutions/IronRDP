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

use ironrdp_bench::connector_replay::{ConnectorReplayId, ConnectorReplayWorkload};
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
    let mut arguments = pico_args::Arguments::from_env();
    if arguments.contains(["-h", "--help"]) {
        if !arguments.finish().is_empty() {
            return Err(usage().to_owned());
        }
        println!("{}", usage());
        return Ok(());
    }
    let capture: Option<String> = arguments
        .opt_value_from_str("--capture")
        .map_err(|_| usage().to_owned())?;
    let connector: Option<String> = arguments
        .opt_value_from_str("--connector")
        .map_err(|_| usage().to_owned())?;
    if !arguments.finish().is_empty() {
        return Err(usage().to_owned());
    }

    match (capture, connector) {
        (Some(selector), None) => {
            let id = PartialReplayId::from_str(&selector).map_err(|error| error.to_string())?;
            let workload = PartialReplayWorkload::prepare(id).map_err(|error| error.to_string())?;
            let measurement = workload.verify().map_err(|error| error.to_string())?;
            println!(
                "workload=partial-replay/{}\trouted_pdus={}\tgraphics_updates={}",
                workload.id().as_str(),
                measurement.routed_pdus,
                measurement.graphics_updates
            );
        }
        (None, Some(selector)) => {
            let id = ConnectorReplayId::from_str(&selector).map_err(|error| error.to_string())?;
            let workload = ConnectorReplayWorkload::prepare(id).map_err(|error| error.to_string())?;
            let measurement = workload.verify().map_err(|error| error.to_string())?;
            println!(
                "workload=connector-replay/{}/connection-and-session\tconnector_steps={}\tactive_frames={}\tactive_x224_frames={}\tactive_fast_path_frames={}\tgraphics_updates={}",
                workload.id().as_str(),
                measurement.connector_steps,
                measurement.active_frames,
                measurement.active_x224_frames,
                measurement.active_fast_path_frames,
                measurement.graphics_updates
            );
        }
        (None, None) | (Some(_), Some(_)) => return Err(usage().to_owned()),
    }
    Ok(())
}

const fn usage() -> &'static str {
    "usage: capture-replay-bench (--capture <no-nla-accepted|no-nla-smartcard> | --connector <no-nla-accepted>)"
}
