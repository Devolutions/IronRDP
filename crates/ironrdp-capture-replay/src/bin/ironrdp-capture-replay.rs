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

use ironrdp_capture_replay::{
    ExportOptions, ReplayDirection, ReplayGap, ReplayGapKind, ReplayGapReason, ReplayLifecycle, ReplayOptions,
    export_capture, prepare_capture, read_capture,
};
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
        Err(error @ (ironrdp_capture_replay::ReplayError::Io(_) | ironrdp_capture_replay::ReplayError::Pcap(_))) => {
            return Err(error.into());
        }
        Err(error) if arguments.summary => {
            print_summary_error(error);
            return Ok(());
        }
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
                if arguments.show_gaps {
                    print_gaps(&execution.report.gaps);
                }
                Ok(())
            }
            Err(error) => {
                print_summary_error(error);
                Ok(())
            }
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
    show_gaps: bool,
}

fn parse_arguments(arguments: impl Iterator<Item = OsString>) -> Result<Arguments, String> {
    let mut replace = false;
    let mut summary = false;
    let mut show_gaps = false;
    let mut key_log = None;
    let mut paths = Vec::new();
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        if argument == "--replace" {
            replace = true;
        } else if argument == "--summary" {
            summary = true;
        } else if argument == "--gaps" {
            show_gaps = true;
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
        show_gaps,
    })
}

fn usage() -> String {
    "usage: ironrdp-capture-replay [--replace] [--keylog <tls-keys.log>] <capture.pcapng> <output-directory>\n       ironrdp-capture-replay --summary [--gaps] [--keylog <tls-keys.log>] <capture.pcapng>".to_owned()
}

fn print_summary_error(error: ironrdp_capture_replay::ReplayError) {
    let (stage, reason) = error.summary_code();
    println!("status=error\tstage={stage}\treason={reason}");
}

fn print_summary(summary: &ironrdp_capture_replay::ReplaySummary) {
    let dimensions = summary
        .final_dimensions
        .map_or_else(|| "-".to_owned(), |(width, height)| format!("{width}x{height}"));
    let fingerprint = summary.output_fingerprint.map_or_else(
        || "-".to_owned(),
        |bytes| bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
    );
    let gap_fingerprint = summary
        .gap_fingerprint
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    println!(
        "status=ok\tclient-pdus={}\tserver-pdus={}\tconnection-pdus={}\tclient-observation-pdus={}\tfast-path-pdus={}\tio-channel-pdus={}\tmessage-channel-pdus={}\tstatic-channel-pdus={}\tother-server-message-pdus={}\tgraphics-updates={}\tfinal-dimensions={dimensions}\tfingerprint={fingerprint}\tlifecycle={}\tframing-gaps={}\ttruncated-pdu-gaps={}\tstatic-channel-gaps={}\tdynamic-channel-gaps={}\tsession-gaps={}\tincomplete-activation-gaps={}\tunsupported-gaps={}\tgap-fingerprint={gap_fingerprint}",
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
        lifecycle_name(summary.lifecycle),
        summary.framing_gaps,
        summary.truncated_pdu_gaps,
        summary.static_channel_gaps,
        summary.dynamic_channel_gaps,
        summary.session_gaps,
        summary.incomplete_activation_gaps,
        summary.unsupported_gaps,
    );
}

fn lifecycle_name(lifecycle: ReplayLifecycle) -> &'static str {
    match lifecycle {
        ReplayLifecycle::NeverActivated => "never-activated",
        ReplayLifecycle::Active => "active",
        ReplayLifecycle::Deactivated => "deactivated",
    }
}

fn print_gaps(gaps: &[ReplayGap]) {
    const MAX_GAP_DETAILS: usize = 16;

    for gap in gaps.iter().take(MAX_GAP_DETAILS) {
        println!(
            "gap=packet:{}\tdirection:{}\tkind:{}\treason:{}\tskipped-bytes:{}",
            gap.packet,
            direction_name(gap.direction),
            gap_kind_name(gap.kind),
            gap_reason_name(gap.reason),
            gap.skipped_bytes,
        );
    }
    if gaps.len() > MAX_GAP_DETAILS {
        println!("gap-details-truncated={}", gaps.len() - MAX_GAP_DETAILS);
    }
}

fn direction_name(direction: ReplayDirection) -> &'static str {
    match direction {
        ReplayDirection::Client => "client",
        ReplayDirection::Server => "server",
    }
}

fn gap_kind_name(kind: ReplayGapKind) -> &'static str {
    match kind {
        ReplayGapKind::Framing => "framing",
        ReplayGapKind::TruncatedPdu => "truncated-pdu",
        ReplayGapKind::StaticChannel => "static-channel",
        ReplayGapKind::DynamicChannel => "dynamic-channel",
        ReplayGapKind::Session => "session",
        ReplayGapKind::IncompleteActivation => "incomplete-activation",
        ReplayGapKind::Unsupported => "unsupported",
    }
}

fn gap_reason_name(reason: ReplayGapReason) -> &'static str {
    match reason {
        ReplayGapReason::Framing => "framing",
        ReplayGapReason::TruncatedPdu => "truncated-pdu",
        ReplayGapReason::StaticChannelPdu => "static-channel-pdu",
        ReplayGapReason::StaticChannelEncode => "static-channel-encode",
        ReplayGapReason::StaticChannelDecode => "static-channel-decode",
        ReplayGapReason::StaticChannelBulkDecompression => "static-channel-bulk-decompression",
        ReplayGapReason::StaticChannelBitmapSourceLength => "static-channel-bitmap-source-length",
        ReplayGapReason::StaticChannelProcessor => "static-channel-processor",
        ReplayGapReason::StaticChannelOther => "static-channel-other",
        ReplayGapReason::DynamicChannel => "dynamic-channel",
        ReplayGapReason::Session => "session",
        ReplayGapReason::IncompleteActivation => "incomplete-activation",
        ReplayGapReason::Unsupported => "unsupported",
    }
}
