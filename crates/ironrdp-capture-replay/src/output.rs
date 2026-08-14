use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::routing::{ReplayFrame, prepare_replay_capture};
use crate::{Capture, ReplayDirection, ReplayError, ReplayEvent, ReplayGap, ReplayGapKind, ReplayReport, ReplayRoute};

/// Options that control replay artifact export.
#[derive(Clone, Debug)]
pub struct ExportOptions {
    /// Directory that receives the completed export.
    pub directory: PathBuf,
    /// Replace an existing non-empty output directory.
    pub replace: bool,
}

/// Completed replay export metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportSummary {
    /// Directory containing the completed export.
    pub directory: PathBuf,
    /// Number of framebuffer PNG files written.
    pub frame_count: usize,
}

/// Errors that prevent a replay export from completing.
#[derive(Debug, Error)]
pub enum ExportError {
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
    #[error("failed to clean up incomplete replay output")]
    CleanupOutput(#[source] io::Error),
}

/// Replay a capture and write visual frames with payload-free diagnostics.
///
/// The destination receives nothing unless every PNG and tabular file was written successfully.
pub fn export_capture(capture: &Capture, options: &ExportOptions) -> Result<ExportSummary, ExportError> {
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
                fs::remove_dir_all(&output.directory).map_err(ExportError::CleanupOutput)?;
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
    let Some(name) = directory.file_name() else {
        return Err(ExportError::OutputPathNotLeaf);
    };
    if name == "." || name == ".." {
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

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use ironrdp_pdu::Action;

    use super::*;
    use crate::CapturedDynamicChannel;

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn writes_ordered_pngs_and_safe_diagnostics() {
        let directory = temporary_directory("ordered");
        let staging = create_staging_directory(directory.parent().unwrap(), &directory).unwrap();
        let mut output = StagedOutput::new(staging.clone());
        let report = report();
        output.write_frame(frame(12, [0x11, 0x22, 0x33, 0xff])).unwrap();
        output.write_frame(frame(47, [0x44, 0x55, 0x66, 0xff])).unwrap();
        output.write_diagnostics(&report).unwrap();
        replace_output_directory(&staging, &directory, false).unwrap();

        assert_png_rgba(&directory.join("frame_000000.png"), [0x11, 0x22, 0x33, 0xff]);
        assert_png_rgba(&directory.join("frame_000001.png"), [0x44, 0x55, 0x66, 0xff]);
        let diagnostics = fs::read_to_string(directory.join("events.tsv")).unwrap();
        let metadata = fs::read_to_string(directory.join("frame_meta.psv")).unwrap();
        let channels = fs::read_to_string(directory.join("dynamic-channels.tsv")).unwrap();
        assert!(diagnostics.contains("1\t12\tserver\tFastPath\tfast-path"));
        assert_eq!(
            metadata,
            "FrameIndex|FramePacket|FrameSize|FrameFile|UpdatePos|UpdateSize\n\
             0|12|1x1|frame_000000.png|0x0|1x1\n\
             1|47|1x1|frame_000001.png|0x0|1x1\n"
        );
        assert_eq!(channels, "id\n7\n");
        for text in [diagnostics, metadata, channels] {
            assert!(!text.contains("CLIENT_RANDOM"));
            assert!(!text.contains("decrypted payload"));
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn refuses_non_empty_output_without_replace() {
        let directory = temporary_directory("refuse");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("keep"), "existing").unwrap();

        let error = validate_output_directory(&directory, false).unwrap_err();

        assert!(matches!(error, ExportError::OutputDirectoryNotEmpty));
        assert_eq!(fs::read_to_string(directory.join("keep")).unwrap(), "existing");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_non_leaf_output_paths() {
        let error = validate_output_directory(Path::new("."), false).unwrap_err();

        assert!(matches!(error, ExportError::OutputPathNotLeaf));
    }

    #[test]
    fn does_not_replace_new_contents_without_replace() {
        let directory = temporary_directory("race");
        fs::create_dir(&directory).unwrap();
        let staging = create_staging_directory(directory.parent().unwrap(), &directory).unwrap();
        fs::write(directory.join("keep"), "new").unwrap();

        let error = replace_output_directory(&staging, &directory, false).unwrap_err();

        assert!(matches!(error, ExportError::OutputDirectoryNotEmpty));
        assert_eq!(fs::read_to_string(directory.join("keep")).unwrap(), "new");
        fs::remove_dir_all(staging).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn replaces_non_empty_output_after_staging() {
        let directory = temporary_directory("replace");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("keep"), "old").unwrap();
        let staging = create_staging_directory(directory.parent().unwrap(), &directory).unwrap();
        let mut output = StagedOutput::new(staging);
        output.write_frame(frame(12, [0x11, 0x22, 0x33, 0xff])).unwrap();

        let summary = finalize_staged_output(
            &mut output,
            &report(),
            &ExportOptions {
                directory: directory.clone(),
                replace: true,
            },
        )
        .unwrap();

        assert_eq!(summary.frame_count, 1);
        assert!(!directory.join("keep").exists());
        assert!(directory.join("frame_000000.png").exists());
        let backup_prefix = format!(".{}.previous-", directory.file_name().unwrap().to_string_lossy());
        assert!(
            fs::read_dir(directory.parent().unwrap())
                .unwrap()
                .all(|entry| { !entry.unwrap().file_name().to_string_lossy().starts_with(&backup_prefix) })
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn refuses_to_finalize_without_visual_frames() {
        let directory = temporary_directory("no-frames");
        let staging = create_staging_directory(directory.parent().unwrap(), &directory).unwrap();
        let mut output = StagedOutput::new(staging.clone());

        let error = finalize_staged_output(
            &mut output,
            &report(),
            &ExportOptions {
                directory: directory.clone(),
                replace: false,
            },
        )
        .unwrap_err();

        assert!(matches!(error, ExportError::NoVisualFrames));
        assert!(!directory.exists());
        fs::remove_dir_all(staging).unwrap();
    }

    fn report() -> ReplayReport {
        ReplayReport {
            events: vec![ReplayEvent {
                packet: 12,
                direction: ReplayDirection::Server,
                action: Action::FastPath,
                route: ReplayRoute::FastPath,
            }],
            gaps: vec![ReplayGap {
                packet: 20,
                direction: ReplayDirection::Server,
                kind: ReplayGapKind::Framing,
                skipped_bytes: 3,
            }],
            dynamic_channels: vec![CapturedDynamicChannel {
                id: 7,
                name: "CLIENT_RANDOM decrypted payload".to_owned(),
            }],
        }
    }

    fn frame(packet: usize, pixels: [u8; 4]) -> ReplayFrame {
        ReplayFrame {
            packet,
            width: 1,
            height: 1,
            pixels: pixels.to_vec(),
        }
    }

    fn assert_png_rgba(path: &Path, expected: [u8; 4]) {
        let input = fs::File::open(path).unwrap();
        let decoder = png::Decoder::new(io::BufReader::new(input));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();

        assert_eq!((info.width, info.height), (1, 1));
        assert_eq!(&pixels[..info.buffer_size()], expected);
    }

    fn temporary_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ironrdp-capture-replay-{name}-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
        ))
    }
}
