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

use ironrdp_capture_replay::{ExportOptions, ReplayOptions, export_capture, prepare_capture, read_capture};
use zeroize::Zeroize as _;

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
    let mut capture = match read_capture(&arguments.capture) {
        Ok(capture) => capture,
        Err(error) if arguments.summary => return print_summary_error(error),
        Err(error) => return Err(error.into()),
    };
    if let Some(key_log) = &arguments.key_log {
        let mut key_log = std::fs::read_to_string(key_log)
            .map_err(|error| usage_error(format!("failed to read {}: {error}", key_log.display())))?;
        capture.add_tls_key_log(&key_log);
        key_log.zeroize();
    }
    if arguments.summary {
        return match prepare_capture(&capture).and_then(|prepared| {
            prepared.replay_with_options(ReplayOptions {
                calculate_output_fingerprint: true,
            })
        }) {
            Ok(execution) => {
                print_summary(&execution.summary);
                Ok(())
            }
            Err(error) => print_summary_error(error),
        };
    }
    let summary = export_capture(
        &capture,
        &ExportOptions {
            directory: arguments.output.expect("export mode requires an output directory"),
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
    output: Option<PathBuf>,
    key_log: Option<PathBuf>,
    replace: bool,
    summary: bool,
}

fn parse_arguments(arguments: impl Iterator<Item = OsString>) -> Result<Arguments, String> {
    let mut replace = false;
    let mut summary = false;
    let mut key_log = None;
    let mut paths = Vec::new();
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        if argument == "--replace" {
            replace = true;
        } else if argument == "--summary" {
            summary = true;
        } else if argument == "--keylog" {
            key_log = Some(PathBuf::from(arguments.next().ok_or_else(usage)?));
        } else if argument.to_string_lossy().starts_with('-') {
            return Err(usage());
        } else {
            paths.push(argument);
        }
    }
    let (capture, output) = if summary {
        let [capture]: [OsString; 1] = paths.try_into().map_err(|_| usage())?;
        (capture, None)
    } else {
        let [capture, output]: [OsString; 2] = paths.try_into().map_err(|_| usage())?;
        (capture, Some(PathBuf::from(output)))
    };

    Ok(Arguments {
        capture: PathBuf::from(capture),
        output,
        key_log,
        replace,
        summary,
    })
}

fn usage() -> String {
    "usage: ironrdp-capture-replay [--replace] [--keylog <tls-keys.log>] <capture.pcapng> <output-directory>\n       ironrdp-capture-replay --summary [--keylog <tls-keys.log>] <capture.pcapng>".to_owned()
}

fn print_summary_error(error: ironrdp_capture_replay::ReplayError) -> Result<(), Box<dyn core::error::Error>> {
    let (stage, reason) = error.summary_code();
    println!("status=error\tstage={stage}\treason={reason}");
    Ok(())
}

fn print_summary(summary: &ironrdp_capture_replay::ReplaySummary) {
    let dimensions = summary
        .final_dimensions
        .map_or_else(|| "-".to_owned(), |(width, height)| format!("{width}x{height}"));
    let fingerprint = summary.output_fingerprint.map_or_else(
        || "-".to_owned(),
        |bytes| bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
    );
    println!(
        "status=ok\tclient-pdus={}\tserver-pdus={}\tconnection-pdus={}\tclient-observation-pdus={}\tfast-path-pdus={}\tio-channel-pdus={}\tmessage-channel-pdus={}\tstatic-channel-pdus={}\tother-server-message-pdus={}\tgraphics-updates={}\tfinal-dimensions={dimensions}\tfingerprint={fingerprint}\tframing-gaps={}\ttruncated-pdu-gaps={}\tstatic-channel-gaps={}\tdynamic-channel-gaps={}\tsession-gaps={}\tincomplete-activation-gaps={}\tunsupported-gaps={}",
        summary.client_pdus,
        summary.server_pdus,
        summary.connection_pdus,
        summary.client_observation_pdus,
        summary.fast_path_pdus,
        summary.io_channel_pdus,
        summary.message_channel_pdus,
        summary.static_channel_pdus,
        summary.other_server_message_pdus,
        summary.graphics_updates,
        summary.framing_gaps,
        summary.truncated_pdu_gaps,
        summary.static_channel_gaps,
        summary.dynamic_channel_gaps,
        summary.session_gaps,
        summary.incomplete_activation_gaps,
        summary.unsupported_gaps,
    );
}
