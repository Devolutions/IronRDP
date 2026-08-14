#![expect(
    unused_crate_dependencies,
    reason = "the binary delegates replay implementation to the library crate"
)]
#![expect(
    clippy::print_stdout,
    reason = "the command reports a completed export path to its interactive caller"
)]
#![expect(
    clippy::print_stderr,
    reason = "the command reports explicit failures to its interactive caller"
)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use ironrdp_capture_replay::{ExportOptions, export_capture, read_capture};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("replay export failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments = parse_arguments(std::env::args_os().skip(1))?;
    let capture = read_capture(&arguments.capture).map_err(|error| error.to_string())?;
    let summary = export_capture(
        &capture,
        &ExportOptions {
            directory: arguments.output,
            replace: arguments.replace,
        },
    )
    .map_err(|error| error.to_string())?;

    println!(
        "exported {} replay frame(s) to {}",
        summary.frame_count,
        summary.directory.display()
    );
    Ok(())
}

struct Arguments {
    capture: PathBuf,
    output: PathBuf,
    replace: bool,
}

fn parse_arguments(arguments: impl Iterator<Item = OsString>) -> Result<Arguments, String> {
    let mut replace = false;
    let mut paths = Vec::new();
    for argument in arguments {
        if argument == "--replace" {
            replace = true;
        } else if argument.to_string_lossy().starts_with('-') {
            return Err(usage());
        } else {
            paths.push(argument);
        }
    }
    let [capture, output]: [OsString; 2] = paths.try_into().map_err(|_| usage())?;

    Ok(Arguments {
        capture: PathBuf::from(capture),
        output: PathBuf::from(output),
        replace,
    })
}

fn usage() -> String {
    "usage: ironrdp-capture-replay [--replace] <capture.pcapng> <output-directory>".to_owned()
}
