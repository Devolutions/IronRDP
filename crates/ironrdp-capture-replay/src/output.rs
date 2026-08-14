use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::{ReplayDirection, ReplayEvent, ReplayFrame, ReplayGap, ReplayGapKind, ReplayReport, ReplayRoute};

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
    /// Replacing output was not explicitly approved.
    #[error("replay output directory is not empty; use --replace to replace it")]
    OutputDirectoryNotEmpty,
    /// A routed frame is not a complete RGBA framebuffer.
    #[error("replay frame {frame} has invalid RGBA pixel data")]
    InvalidFrame {
        /// Index of the invalid frame in replay order.
        frame: usize,
    },
    /// Frame order would not preserve capture provenance.
    #[error("replay frame {current} follows packet {previous} out of capture order")]
    FrameOutOfOrder {
        /// Packet number of the preceding frame.
        previous: usize,
        /// Packet number of the out-of-order frame.
        current: usize,
    },
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

/// Write visual replay frames and payload-free diagnostics into one output directory.
///
/// The destination receives nothing unless all PNGs and tabular files were written successfully.
pub fn export_replay(report: &ReplayReport, options: &ExportOptions) -> Result<ExportSummary, ExportError> {
    validate_frames(&report.frames)?;
    validate_output_directory(&options.directory, options.replace)?;

    let parent = output_parent(&options.directory);
    fs::create_dir_all(parent).map_err(ExportError::PrepareOutput)?;
    let staging = create_staging_directory(parent, &options.directory)?;

    let result = write_export(&staging, report).and_then(|()| replace_output_directory(&staging, &options.directory));
    if let Err(error) = result {
        fs::remove_dir_all(&staging).map_err(ExportError::CleanupOutput)?;
        return Err(error);
    }

    Ok(ExportSummary {
        directory: options.directory.clone(),
        frame_count: report.frames.len(),
    })
}

fn validate_frames(frames: &[ReplayFrame]) -> Result<(), ExportError> {
    if frames.is_empty() {
        return Err(ExportError::NoVisualFrames);
    }

    let mut previous_packet = None;
    for (index, frame) in frames.iter().enumerate() {
        if let Some(previous) = previous_packet
            && frame.packet < previous
        {
            return Err(ExportError::FrameOutOfOrder {
                previous,
                current: frame.packet,
            });
        }
        previous_packet = Some(frame.packet);

        let expected_len = usize::from(frame.width)
            .checked_mul(usize::from(frame.height))
            .and_then(|pixels| pixels.checked_mul(4 /* RGBA */));
        if frame.width == 0 || frame.height == 0 || expected_len != Some(frame.pixels.len()) {
            return Err(ExportError::InvalidFrame { frame: index + 1 });
        }
    }

    Ok(())
}

fn validate_output_directory(directory: &Path, replace: bool) -> Result<(), ExportError> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(ExportError::OutputPathNotDirectory)
        }
        Ok(_) => {
            let is_empty = fs::read_dir(directory)
                .map_err(ExportError::PrepareOutput)?
                .next()
                .transpose()
                .map_err(ExportError::PrepareOutput)?
                .is_none();
            if is_empty || replace {
                Ok(())
            } else {
                Err(ExportError::OutputDirectoryNotEmpty)
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ExportError::PrepareOutput(error)),
    }
}

fn output_parent(directory: &Path) -> &Path {
    directory
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn create_staging_directory(parent: &Path, directory: &Path) -> Result<PathBuf, ExportError> {
    let stem = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("replay-output");
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

fn write_export(directory: &Path, report: &ReplayReport) -> Result<(), ExportError> {
    for (index, frame) in report.frames.iter().enumerate() {
        let name = format!("frame-{index:06}-packet-{:012}.png", frame.packet);
        let png = encode_png(frame)?;
        fs::write(directory.join(name), png).map_err(ExportError::WriteOutput)?;
    }

    fs::write(directory.join("metadata.tsv"), metadata_tsv(report)).map_err(ExportError::WriteOutput)?;
    fs::write(directory.join("events.tsv"), events_tsv(&report.events)).map_err(ExportError::WriteOutput)?;
    fs::write(directory.join("gaps.tsv"), gaps_tsv(&report.gaps)).map_err(ExportError::WriteOutput)?;
    fs::write(directory.join("dynamic-channels.tsv"), dynamic_channels_tsv(report))
        .map_err(ExportError::WriteOutput)?;
    Ok(())
}

fn replace_output_directory(staging: &Path, directory: &Path) -> Result<(), ExportError> {
    match fs::symlink_metadata(directory) {
        Ok(_) => fs::remove_dir_all(directory).map_err(ExportError::FinalizeOutput)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(ExportError::FinalizeOutput(error)),
    }
    fs::rename(staging, directory).map_err(ExportError::FinalizeOutput)
}

fn encode_png(frame: &ReplayFrame) -> Result<Vec<u8>, ExportError> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, u32::from(frame.width), u32::from(frame.height));
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(ExportError::EncodePng)?;
    writer.write_image_data(&frame.pixels).map_err(ExportError::EncodePng)?;
    writer.finish().map_err(ExportError::EncodePng)?;
    Ok(png)
}

fn metadata_tsv(report: &ReplayReport) -> String {
    format!(
        "field\tvalue\nformat-version\t1\nframes\t{}\nevents\t{}\ngaps\t{}\ndynamic-channels\t{}\n",
        report.frames.len(),
        report.events.len(),
        report.gaps.len(),
        report.dynamic_channels.len(),
    )
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
        let report = report_with_frames();

        let summary = export_replay(
            &report,
            &ExportOptions {
                directory: directory.clone(),
                replace: false,
            },
        )
        .unwrap();

        assert_eq!(summary.frame_count, 2);
        let first = directory.join("frame-000000-packet-000000000012.png");
        let second = directory.join("frame-000001-packet-000000000047.png");
        assert!(first.is_file());
        assert!(second.is_file());
        assert_png_rgba(&first, [0x11, 0x22, 0x33, 0xff]);
        assert_png_rgba(&second, [0x44, 0x55, 0x66, 0xff]);

        let diagnostics = fs::read_to_string(directory.join("events.tsv")).unwrap();
        let metadata = fs::read_to_string(directory.join("metadata.tsv")).unwrap();
        let channels = fs::read_to_string(directory.join("dynamic-channels.tsv")).unwrap();
        assert!(diagnostics.contains("1\t12\tserver\tFastPath\tfast-path"));
        assert!(metadata.contains("frames\t2"));
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

        let error = export_replay(
            &report_with_frames(),
            &ExportOptions {
                directory: directory.clone(),
                replace: false,
            },
        )
        .unwrap_err();

        assert!(matches!(error, ExportError::OutputDirectoryNotEmpty));
        assert_eq!(fs::read_to_string(directory.join("keep")).unwrap(), "existing");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn replacement_removes_existing_contents_after_staging() {
        let directory = temporary_directory("replace");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("stale"), "existing").unwrap();

        export_replay(
            &report_with_frames(),
            &ExportOptions {
                directory: directory.clone(),
                replace: true,
            },
        )
        .unwrap();

        assert!(!directory.join("stale").exists());
        assert!(directory.join("metadata.tsv").is_file());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn refuses_reports_without_visual_frames() {
        let directory = temporary_directory("no-frames");
        let error = export_replay(
            &ReplayReport::default(),
            &ExportOptions {
                directory: directory.clone(),
                replace: false,
            },
        )
        .unwrap_err();

        assert!(matches!(error, ExportError::NoVisualFrames));
        assert!(!directory.exists());
    }

    fn report_with_frames() -> ReplayReport {
        ReplayReport {
            events: vec![
                ReplayEvent {
                    packet: 12,
                    direction: ReplayDirection::Server,
                    action: Action::FastPath,
                    route: ReplayRoute::FastPath,
                },
                ReplayEvent {
                    packet: 47,
                    direction: ReplayDirection::Server,
                    action: Action::X224,
                    route: ReplayRoute::IoChannel,
                },
            ],
            frames: vec![
                ReplayFrame {
                    packet: 12,
                    width: 1,
                    height: 1,
                    pixels: vec![0x11, 0x22, 0x33, 0xff],
                },
                ReplayFrame {
                    packet: 47,
                    width: 1,
                    height: 1,
                    pixels: vec![0x44, 0x55, 0x66, 0xff],
                },
            ],
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
