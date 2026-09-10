#![allow(unused_crate_dependencies)] // The package also contains Criterion benchmark targets.
#![allow(clippy::print_stdout)]

use core::num::{NonZeroU16, NonZeroUsize};
use core::time::Duration;
use std::io;
use std::time::Instant;

use anyhow::Context as _;
use ironrdp::pdu::rdp::capability_sets::{CmdFlags, EntropyBits, LargePointerSupportFlags};
use ironrdp::server::bench::encoder::{UpdateEncoder, UpdateEncoderCodecs};
use ironrdp::server::{BitmapUpdate, DesktopSize, DisplayUpdate, PixelFormat, RdpServerDisplayUpdates};
use tokio::fs::File;
use tokio::io::AsyncReadExt as _;
use tokio::time::sleep;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    setup_logging()?;
    let mut args = pico_args::Arguments::from_env();

    if args.contains(["-h", "--help"]) {
        println!("Usage: perfenc [OPTIONS] <RGBX_INPUT_FILENAME>");
        println!();
        println!("Encode headerless RGBX frames with one persistent server encoder.");
        println!("The default is quiet and unpaced for whole-process measurements.");
        println!();
        println!("Options:");
        println!("  --width <WIDTH>      Width of the display (default: 3840)");
        println!("  --height <HEIGHT>    Height of the display (default: 2400)");
        println!("  --codec <CODEC>      Codec to use (default: remotefx)");
        println!("                        Valid values: qoi, qoiz, remotefx, bitmap, none");
        println!("  --fps <FPS>          Limit frame delivery for interactive use (default: none)");
        return Ok(());
    }

    let width = args.opt_value_from_str("--width")?.unwrap_or(3840);
    let height = args.opt_value_from_str("--height")?.unwrap_or(2400);
    let codec = args.opt_value_from_str("--codec")?.unwrap_or_else(OptCodec::default);
    let fps = args.opt_value_from_str("--fps")?.unwrap_or(0);

    let filename: String = args.free_from_str().context("missing RGBX input filename")?;
    let file = File::open(&filename)
        .await
        .with_context(|| format!("failed to open file: {filename}"))?;
    let desktop_size = DesktopSize { width, height };
    let mut encoder = create_encoder(desktop_size, codec)?;
    let mut updates = DisplayUpdates::new(file, desktop_size, fps);

    let mut total_raw = 0u64;
    let mut total_encoded = 0u64;
    let mut update_count = 0u64;
    let mut fragment_count = 0u64;
    while let Some(update) = updates.next_update().await? {
        let raw_size = match &update {
            DisplayUpdate::Bitmap(bitmap) => bitmap.data.len(),
            _ => anyhow::bail!("RGBX source produced a non-bitmap update"),
        };

        total_raw += u64::try_from(raw_size).context("frame size does not fit in u64")?;
        let mut fragments = encoder.update(update);
        while let Some(fragment) = fragments.next().await {
            total_encoded +=
                u64::try_from(fragment?.data.len()).context("encoded fragment size does not fit in u64")?;
            fragment_count += 1;
        }
        update_count += 1;
    }

    anyhow::ensure!(update_count > 0, "RGBX input contains no complete frames");
    anyhow::ensure!(fragment_count > 0, "encoder produced no output fragments");
    anyhow::ensure!(total_encoded > 0, "encoder produced no output bytes");

    #[expect(clippy::as_conversions, reason = "u64-to-f64 conversion is used only for reporting")]
    let reduction = 100.0 - (total_encoded as f64 / total_raw as f64) * 100.0;
    println!(
        "updates={update_count}\tfragments={fragment_count}\traw_bytes={total_raw}\tencoded_bytes={total_encoded}\treduction={reduction:.2}%"
    );

    Ok(())
}

fn create_encoder(desktop_size: DesktopSize, codec: OptCodec) -> anyhow::Result<UpdateEncoder> {
    let mut flags = CmdFlags::all();
    let mut codecs = UpdateEncoderCodecs::new();

    match codec {
        OptCodec::RemoteFX => codecs.set_remotefx(Some((EntropyBits::Rlgr3, 0))),
        OptCodec::Bitmap => flags -= CmdFlags::SET_SURFACE_BITS,
        OptCodec::None => {}
        #[cfg(feature = "qoi")]
        OptCodec::Qoi => codecs.set_qoi(Some(0)),
        #[cfg(feature = "qoiz")]
        OptCodec::QoiZ => codecs.set_qoiz(Some(0)),
    };

    UpdateEncoder::new(
        desktop_size,
        flags,
        codecs,
        8 * 1024 * 1024,
        u16::MAX,
        LargePointerSupportFlags::all(),
    )
    .context("failed to initialize update encoder")
}

struct DisplayUpdates {
    file: File,
    desktop_size: DesktopSize,
    fps: u32,
    last_update_time: Option<Instant>,
}

impl DisplayUpdates {
    fn new(file: File, desktop_size: DesktopSize, fps: u32) -> Self {
        Self {
            file,
            desktop_size,
            fps,
            last_update_time: None,
        }
    }
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for DisplayUpdates {
    async fn next_update(&mut self) -> ironrdp::server::ServerResult<Option<DisplayUpdate>> {
        use ironrdp::server::ServerErrorExt as _;

        let stride = usize::from(self.desktop_size.width) * 4;
        let frame_size = stride * usize::from(self.desktop_size.height);
        let mut frame = vec![0; frame_size];
        match read_frame(&mut self.file, &mut frame)
            .await
            .map_err(|error| ironrdp::server::ServerError::io("read RGBX frame", error))?
        {
            FrameRead::EndOfFile => return Ok(None),
            FrameRead::Truncated => {
                return Err(ironrdp::server::ServerError::reason("perfenc", "truncated RGBX frame"));
            }
            FrameRead::Complete => {}
        }

        if self.fps > 0 {
            let now = Instant::now();
            if let Some(last_update_time) = self.last_update_time {
                let frame_interval = Duration::from_secs(1) / self.fps;
                let elapsed = now - last_update_time;
                if elapsed < frame_interval {
                    sleep(frame_interval - elapsed).await;
                }
            }
            self.last_update_time = Some(Instant::now());
        }

        Ok(Some(DisplayUpdate::Bitmap(BitmapUpdate {
            x: 0,
            y: 0,
            width: NonZeroU16::new(self.desktop_size.width)
                .ok_or_else(|| ironrdp::server::ServerError::reason("perfenc", "width cannot be zero"))?,
            height: NonZeroU16::new(self.desktop_size.height)
                .ok_or_else(|| ironrdp::server::ServerError::reason("perfenc", "height cannot be zero"))?,
            format: PixelFormat::RgbX32,
            data: frame.into(),
            stride: NonZeroUsize::new(stride)
                .ok_or_else(|| ironrdp::server::ServerError::reason("perfenc", "stride cannot be zero"))?,
        })))
    }
}

enum FrameRead {
    Complete,
    EndOfFile,
    Truncated,
}

async fn read_frame(file: &mut File, frame: &mut [u8]) -> io::Result<FrameRead> {
    let mut filled = 0;
    while filled < frame.len() {
        let read = file.read(&mut frame[filled..]).await?;
        if read == 0 {
            return Ok(if filled == 0 {
                FrameRead::EndOfFile
            } else {
                FrameRead::Truncated
            });
        }
        filled += read;
    }
    Ok(FrameRead::Complete)
}

fn setup_logging() -> anyhow::Result<()> {
    use tracing::metadata::LevelFilter;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .with_env_var("IRONRDP_LOG")
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().compact())
        .with(env_filter)
        .try_init()
        .context("failed to set tracing global subscriber")?;

    Ok(())
}

#[derive(Default)]
enum OptCodec {
    #[default]
    RemoteFX,
    Bitmap,
    None,
    #[cfg(feature = "qoi")]
    Qoi,
    #[cfg(feature = "qoiz")]
    QoiZ,
}

impl core::str::FromStr for OptCodec {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "remotefx" => Ok(Self::RemoteFX),
            "bitmap" => Ok(Self::Bitmap),
            "none" => Ok(Self::None),
            #[cfg(feature = "qoi")]
            "qoi" => Ok(Self::Qoi),
            #[cfg(feature = "qoiz")]
            "qoiz" => Ok(Self::QoiZ),
            _ => anyhow::bail!("unknown codec: {value}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temporary_path(name: &str) -> std::path::PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ironrdp-perfenc-{name}-{}-{timestamp}", std::process::id()))
    }

    async fn display_updates(bytes: &[u8]) -> (DisplayUpdates, std::path::PathBuf) {
        let path = temporary_path("frame");
        fs::write(&path, bytes).expect("write temporary frame source");
        let file = File::open(&path).await.expect("open temporary frame source");
        (DisplayUpdates::new(file, DesktopSize { width: 1, height: 1 }, 0), path)
    }

    #[tokio::test]
    async fn returns_none_at_clean_eof() {
        let (mut updates, path) = display_updates(&[]).await;
        assert!(updates.next_update().await.expect("clean EOF is valid").is_none());
        fs::remove_file(path).expect("remove temporary frame source");
    }

    #[tokio::test]
    async fn rejects_truncated_frame() {
        let (mut updates, path) = display_updates(&[0, 1, 2]).await;
        let error = updates.next_update().await.expect_err("partial frame must fail");
        assert!(error.to_string().contains("truncated RGBX frame"));
        fs::remove_file(path).expect("remove temporary frame source");
    }
}
