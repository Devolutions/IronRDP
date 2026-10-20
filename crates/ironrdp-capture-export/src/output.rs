use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use ironrdp_capture::Capture;

use ironrdp_capture_replay::{
    ReplayDirection, ReplayError, ReplayEvent, ReplayFrame, ReplayGap, ReplayGapKind, ReplayReport, ReplayRoute,
    prepare_replay_capture,
};

/// Options that control replay artifact export.
#[derive(Clone, Debug)]
pub(crate) struct ExportOptions {
    /// Directory that receives the completed export.
    pub directory: PathBuf,
    /// Replace an existing non-empty output directory.
    pub replace: bool,
}

/// Completed replay export metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExportSummary {
    /// Directory containing the completed export.
    pub directory: PathBuf,
    /// Number of framebuffer PNG files written.
    pub frame_count: usize,
}

/// Errors that prevent a replay export from completing.
#[derive(Debug, Error)]
pub(crate) enum ExportError {
    /// The routed capture had no visual updates to export.
    #[error("replay did not produce any visual frames")]
    NoVisualFrames,
    /// The selected output path cannot contain the export.
    #[error("replay output path is not a directory")]
    OutputPathNotDirectory,
    /// The selected output path does not name a replaceable directory.
    #[error("replay output path must name a directory")]
    OutputPathNotLeaf,
    /// Replacing output was not explicitly approved.
    #[error("replay output directory is not empty; use --replace to replace it")]
    OutputDirectoryNotEmpty,
    /// The captured replay could not be prepared.
    #[error("failed to replay capture")]
    Replay(#[source] ReplayError),
    /// A filesystem operation failed before output was completed.
    #[error("failed to prepare replay output")]
    PrepareOutput(#[source] io::Error),
    /// A filesystem operation failed while producing staged output.
    #[error("failed to write replay output")]
    WriteOutput(#[source] io::Error),
    /// PNG encoding failed.
    #[error("failed to encode replay frame")]
    EncodePng(#[source] png::EncodingError),
    /// The fully staged output could not replace the destination.
    #[error("failed to finalize replay output")]
    FinalizeOutput(#[source] io::Error),
    /// An incomplete staged export could not be removed.
    #[error("{original_error}; additionally failed to clean up incomplete replay output: {cleanup_error}")]
    CleanupOutput {
        #[source]
        original_error: Box<ExportError>,
        cleanup_error: io::Error,
    },
}

/// Replay a capture and write visual frames with payload-free diagnostics.
///
/// The destination receives nothing unless every PNG and tabular file was written successfully.
pub(crate) fn export_capture(capture: &Capture, options: &ExportOptions) -> Result<ExportSummary, ExportError> {
    let (mut router, plaintext) = prepare_replay_capture(capture).map_err(ExportError::Replay)?;
    validate_output_directory(&options.directory, options.replace)?;

    let parent = output_parent(&options.directory);
    fs::create_dir_all(parent).map_err(ExportError::PrepareOutput)?;
    let staging = create_staging_directory(parent, &options.directory)?;
    let mut output = StagedOutput::new(staging);
    let result = router
        .route_plaintext_with_frame_sink(&plaintext, &mut |frame| output.write_frame(frame))
        .and_then(|report| finalize_staged_output(&mut output, &report, options));
    match result {
        Ok(summary) => Ok(summary),
        Err(error) => {
            if !output.finalization_started {
                if let Err(cleanup_error) = fs::remove_dir_all(&output.directory) {
                    return Err(ExportError::CleanupOutput {
                        original_error: Box::new(error),
                        cleanup_error,
                    });
                }
            }
            Err(error)
        }
    }
}

fn finalize_staged_output(
    output: &mut StagedOutput,
    report: &ReplayReport,
    options: &ExportOptions,
) -> Result<ExportSummary, ExportError> {
    if output.frame_count == 0 {
        return Err(ExportError::NoVisualFrames);
    }
    output.write_diagnostics(report)?;
    output.finalization_started = true;
    replace_output_directory(&output.directory, &options.directory, options.replace)?;
    Ok(ExportSummary {
        directory: options.directory.clone(),
        frame_count: output.frame_count,
    })
}

struct StagedOutput {
    directory: PathBuf,
    frame_count: usize,
    frame_metadata: String,
    finalization_started: bool,
}

impl StagedOutput {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            frame_count: 0,
            frame_metadata: String::from("FrameIndex|FramePacket|FrameSize|FrameFile|UpdatePos|UpdateSize\n"),
            finalization_started: false,
        }
    }

    fn write_frame(&mut self, frame: ReplayFrame) -> Result<(), ExportError> {
        let name = format!("frame_{:06}.png", self.frame_count);
        encode_png(&self.directory.join(name), &frame)?;
        self.frame_metadata.push_str(&format!(
            "{}|{}|{}x{}|frame_{:06}.png|0x0|{}x{}\n",
            self.frame_count, frame.packet, frame.width, frame.height, self.frame_count, frame.width, frame.height,
        ));
        self.frame_count += 1;
        Ok(())
    }

    fn write_diagnostics(&self, report: &ReplayReport) -> Result<(), ExportError> {
        fs::write(self.directory.join("frame_meta.psv"), &self.frame_metadata).map_err(ExportError::WriteOutput)?;
        fs::write(self.directory.join("events.tsv"), events_tsv(&report.events)).map_err(ExportError::WriteOutput)?;
        fs::write(self.directory.join("gaps.tsv"), gaps_tsv(&report.gaps)).map_err(ExportError::WriteOutput)?;
        fs::write(
            self.directory.join("dynamic-channels.tsv"),
            dynamic_channels_tsv(report),
        )
        .map_err(ExportError::WriteOutput)?;
        Ok(())
    }
}

fn validate_output_directory(directory: &Path, replace: bool) -> Result<(), ExportError> {
    if directory.file_name().is_none() {
        return Err(ExportError::OutputPathNotLeaf);
    }

    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(ExportError::OutputPathNotDirectory)
        }
        Ok(_) if !replace && !directory_is_empty(directory).map_err(ExportError::PrepareOutput)? => {
            Err(ExportError::OutputDirectoryNotEmpty)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ExportError::PrepareOutput(error)),
    }
}

fn directory_is_empty(directory: &Path) -> io::Result<bool> {
    Ok(fs::read_dir(directory)?.next().transpose()?.is_none())
}

fn output_parent(directory: &Path) -> &Path {
    directory.parent().unwrap_or_else(|| Path::new("."))
}

fn create_staging_directory(parent: &Path, directory: &Path) -> Result<PathBuf, ExportError> {
    let stem = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ExportError::OutputPathNotLeaf)?;
    for attempt in 0..1_000 {
        let staging = parent.join(format!(".{stem}.staging-{}-{attempt}", std::process::id()));
        match fs::create_dir(&staging) {
            Ok(()) => return Ok(staging),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(ExportError::PrepareOutput(error)),
        }
    }

    Err(ExportError::PrepareOutput(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate replay output staging directory",
    )))
}

fn replace_output_directory(staging: &Path, directory: &Path, replace: bool) -> Result<(), ExportError> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(ExportError::OutputPathNotDirectory);
        }
        Ok(_) if replace => {
            let stem = directory
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(ExportError::OutputPathNotLeaf)?;
            let mut previous = None;
            for attempt in 0..1_000 {
                let backup =
                    output_parent(directory).join(format!(".{stem}.previous-{}-{attempt}", std::process::id()));
                match fs::rename(directory, &backup) {
                    Ok(()) => {
                        previous = Some(backup);
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(ExportError::FinalizeOutput(error)),
                }
            }
            let previous = previous.ok_or_else(|| {
                ExportError::FinalizeOutput(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "could not allocate replay output backup directory",
                ))
            })?;

            if let Err(error) = fs::rename(staging, directory) {
                if let Err(restore_error) = fs::rename(&previous, directory) {
                    return Err(ExportError::FinalizeOutput(io::Error::new(
                        error.kind(),
                        format!(
                            "could not publish replay output; previous output remains at {}: {restore_error}",
                            previous.display()
                        ),
                    )));
                }
                return Err(ExportError::FinalizeOutput(io::Error::new(
                    error.kind(),
                    format!(
                        "could not publish replay output; completed export remains staged at {}: {error}",
                        staging.display()
                    ),
                )));
            }

            if let Err(error) = fs::remove_dir_all(&previous) {
                if let Err(rollback_error) = fs::rename(directory, staging) {
                    return Err(ExportError::FinalizeOutput(io::Error::new(
                        error.kind(),
                        format!(
                            "could not remove previous replay output; replay output remains at {} and previous output remains at {}: {rollback_error}",
                            directory.display(),
                            previous.display()
                        ),
                    )));
                }
                if let Err(restore_error) = fs::rename(&previous, directory) {
                    return Err(ExportError::FinalizeOutput(io::Error::new(
                        error.kind(),
                        format!(
                            "could not remove previous replay output; previous output remains at {}: {restore_error}",
                            previous.display()
                        ),
                    )));
                }
                return Err(ExportError::FinalizeOutput(error));
            }
            return Ok(());
        }
        Ok(_) if directory_is_empty(directory).map_err(ExportError::FinalizeOutput)? => {
            fs::remove_dir(directory).map_err(ExportError::FinalizeOutput)?;
        }
        Ok(_) => return Err(ExportError::OutputDirectoryNotEmpty),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(ExportError::FinalizeOutput(error)),
    }
    fs::rename(staging, directory).map_err(ExportError::FinalizeOutput)
}

fn encode_png(path: &Path, frame: &ReplayFrame) -> Result<(), ExportError> {
    let file = fs::File::create(path).map_err(ExportError::WriteOutput)?;
    let mut encoder = png::Encoder::new(file, u32::from(frame.width), u32::from(frame.height));
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(ExportError::EncodePng)?;
    writer.write_image_data(&frame.pixels).map_err(ExportError::EncodePng)?;
    writer.finish().map_err(ExportError::EncodePng)?;
    Ok(())
}

fn events_tsv(events: &[ReplayEvent]) -> String {
    let mut output = String::from("order\tpacket\tdirection\taction\troute\n");
    for (index, event) in events.iter().enumerate() {
        output.push_str(&format!(
            "{}\t{}\t{}\t{:?}\t{}\n",
            index + 1,
            event.packet,
            direction_name(event.direction),
            event.action,
            route_name(event.route),
        ));
    }
    output
}

fn gaps_tsv(gaps: &[ReplayGap]) -> String {
    let mut output = String::from("packet\tdirection\tkind\tskipped-bytes\n");
    for gap in gaps {
        output.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            gap.packet,
            direction_name(gap.direction),
            gap_kind_name(gap.kind),
            gap.skipped_bytes,
        ));
    }
    output
}

fn dynamic_channels_tsv(report: &ReplayReport) -> String {
    let mut output = String::from("id\n");
    for channel in &report.dynamic_channels {
        output.push_str(&format!("{}\n", channel.id));
    }
    output
}

fn direction_name(direction: ReplayDirection) -> &'static str {
    match direction {
        ReplayDirection::Client => "client",
        ReplayDirection::Server => "server",
    }
}

fn route_name(route: ReplayRoute) -> &'static str {
    match route {
        ReplayRoute::Connection => "connection",
        ReplayRoute::ClientObservation => "client-observation",
        ReplayRoute::FastPath => "fast-path",
        ReplayRoute::IoChannel => "io-channel",
        ReplayRoute::MessageChannel => "message-channel",
        ReplayRoute::StaticChannel => "static-channel",
        ReplayRoute::OtherServerMessage => "other-server-message",
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
