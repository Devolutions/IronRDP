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
            let mut source = core::error::Error::source(error.as_ref());
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}

fn usage_error(message: String) -> Box<dyn core::error::Error> {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into()
}

fn run() -> Result<(), Box<dyn core::error::Error>> {
    let arguments = parse_arguments(std::env::args_os().skip(1)).map_err(usage_error)?;
    let mut capture = read_capture(&arguments.capture)?;
    if let Some(key_log) = &arguments.key_log {
        let key_log = std::fs::read_to_string(key_log)
            .map_err(|error| usage_error(format!("failed to read {}: {error}", key_log.display())))?;
        capture.add_tls_key_log(&key_log);
    }
    let summary = export_capture(
        &capture,
        &ExportOptions {
            directory: arguments.output,
            replace: arguments.replace,
        },
    )?;

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
    key_log: Option<PathBuf>,
    replace: bool,
}

fn parse_arguments(arguments: impl Iterator<Item = OsString>) -> Result<Arguments, String> {
    let mut replace = false;
    let mut key_log = None;
    let mut paths = Vec::new();
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        if argument == "--replace" {
            replace = true;
        } else if argument == "--keylog" {
            key_log = Some(PathBuf::from(arguments.next().ok_or_else(usage)?));
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
        key_log,
        replace,
    })
}

fn usage() -> String {
    "usage: ironrdp-capture-replay [--replace] [--keylog <tls-keys.log>] <capture.pcapng> <output-directory>".to_owned()
}
