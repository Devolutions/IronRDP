#![allow(unused_crate_dependencies)] // False positives because there are both a library and a binary.
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
async fn main() -> Result<(), anyhow::Error> {
    setup_logging()?;
    let mut args = pico_args::Arguments::from_env();

    if args.contains(["-h", "--help"]) {
        println!("Usage: perfenc [OPTIONS] <RGBX_INPUT_FILENAME>");
        println!();
        println!("Measure the performance of the IronRDP server encoder, given a raw RGBX video input file.");
        println!();
        println!("Options:");
        println!("  --width <WIDTH>      Width of the display (default: 3840)");
        println!("  --height <HEIGHT>    Height of the display (default: 2400)");
        println!("  --codec <CODEC>      Codec to use (default: remotefx)");
        println!("                        Valid values: qoi, qoiz, remotefx, bitmap, none");
        println!("  --fps <FPS>          Frames per second (default: none)");
        std::process::exit(0);
    }

    let width = NonZeroU16::new(args.opt_value_from_str("--width")?.unwrap_or(3840)).context("width cannot be zero")?;
    let height =
        NonZeroU16::new(args.opt_value_from_str("--height")?.unwrap_or(2400)).context("height cannot be zero")?;
    let codec = args.opt_value_from_str("--codec")?.unwrap_or_else(OptCodec::default);
    let fps = args.opt_value_from_str("--fps")?.unwrap_or(0);

    let filename: String = args.free_from_str().context("missing RGBX input filename")?;
    let file = File::open(&filename)
        .await
        .with_context(|| format!("Failed to open file: {filename}"))?;

    let mut flags = CmdFlags::all();
    let mut update_codecs = UpdateEncoderCodecs::new();

    match codec {
        OptCodec::RemoteFX => update_codecs.set_remotefx(Some((EntropyBits::Rlgr3, 0))),
        OptCodec::Bitmap => {
            flags -= CmdFlags::SET_SURFACE_BITS;
        }
        OptCodec::None => {}
        #[cfg(feature = "qoi")]
        OptCodec::Qoi => update_codecs.set_qoi(Some(0)),
        #[cfg(feature = "qoiz")]
        OptCodec::QoiZ => update_codecs.set_qoiz(Some(0)),
    };

    // u16::MAX / LargePointerSupportFlags::all(): this benchmark replays a fixed update
    // file and isn't exercising the client capability gates, so don't let a
    // no-capability default silently start dropping any pointer updates the replay
    // file happens to contain.
    let mut encoder = UpdateEncoder::new(
        DesktopSize {
            width: width.get(),
            height: height.get(),
        },
        flags,
        update_codecs,
        8 * 1024 * 1024,
        u16::MAX,
        LargePointerSupportFlags::all(),
    )
    .context("failed to initialize update encoder")?;

    let mut total_raw = 0u64;
    let mut total_enc = 0u64;
    let mut n_updates = 0u64;
    let mut n_fragments = 0u64;
    let mut updates = DisplayUpdates::new(file, width, height, fps);
    while let Some(up) = updates.next_update().await? {
        if let DisplayUpdate::Bitmap(ref up) = up {
            total_raw += u64::try_from(up.data.len())?;
        } else {
            anyhow::bail!("invalid update");
        }
        let mut iter = encoder.update(up);
        loop {
            let Some(frag) = iter.next().await else {
                break;
            };
            let len = u64::try_from(frag?.data.len())?;
            total_enc += len;
            n_fragments += 1;
        }
        n_updates += 1;
    }

    anyhow::ensure!(n_updates > 0, "RGBX input contains no complete frames");
    anyhow::ensure!(n_fragments > 0, "encoder produced no output fragments");
    anyhow::ensure!(total_enc > 0, "encoder produced no output bytes");

    #[expect(clippy::as_conversions, reason = "casting u64 to f64")]
    let ratio = total_enc as f64 / total_raw as f64;
    let percent = 100.0 - ratio * 100.0;
    println!(
        "updates={n_updates}\tfragments={n_fragments}\traw_bytes={total_raw}\tencoded_bytes={total_enc}\treduction={percent:.2}%"
    );
    Ok(())
}

struct DisplayUpdates {
    file: File,
    width: NonZeroU16,
    height: NonZeroU16,
    stride: NonZeroUsize,
    fps: u64,
    last_update_time: Option<Instant>,
}

impl DisplayUpdates {
    fn new(file: File, width: NonZeroU16, height: NonZeroU16, fps: u64) -> Self {
        Self {
            file,
            width,
            height,
            stride: NonZeroUsize::new(usize::from(width.get()) * 4).expect("nonzero width has a nonzero RGBX stride"),
            fps,
            last_update_time: None,
        }
    }
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for DisplayUpdates {
    async fn next_update(&mut self) -> ironrdp::server::ServerResult<Option<DisplayUpdate>> {
        use ironrdp::server::ServerErrorExt as _;

        let frame_size = self.stride.get() * usize::from(self.height.get());
        let mut buf = vec![0u8; frame_size];
        // FIXME: read_frame is not cancellation safe; cancelling it loses a partially read frame.
        match read_frame(&mut self.file, &mut buf)
            .await
            .map_err(|e| ironrdp::server::ServerError::io("read frame", e))?
        {
            FrameRead::EndOfFile => return Ok(None),
            FrameRead::Truncated => {
                return Err(ironrdp::server::ServerError::reason("perfenc", "truncated RGBX frame"));
            }
            FrameRead::Complete => {}
        }

        if self.fps > 0 {
            if let Some(last_update_time) = self.last_update_time {
                let frame_interval = Duration::from_nanos(1_000_000_000 / self.fps);
                let elapsed = last_update_time.elapsed();
                if elapsed < frame_interval {
                    sleep(frame_interval - elapsed).await;
                }
            }
            self.last_update_time = Some(Instant::now());
        }

        let up = DisplayUpdate::Bitmap(BitmapUpdate {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
            format: PixelFormat::RgbX32,
            data: buf.into(),
            stride: self.stride,
        });
        Ok(Some(up))
    }
}

enum FrameRead {
    Complete,
    EndOfFile,
    Truncated,
}

/// Fill `frame`, distinguishing a clean end of input from a truncated final frame.
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

    let fmt_layer = tracing_subscriber::fmt::layer().compact();

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .with_env_var("IRONRDP_LOG")
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(fmt_layer)
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

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "remotefx" => Ok(Self::RemoteFX),
            "bitmap" => Ok(Self::Bitmap),
            "none" => Ok(Self::None),
            #[cfg(feature = "qoi")]
            "qoi" => Ok(Self::Qoi),
            #[cfg(feature = "qoiz")]
            "qoiz" => Ok(Self::QoiZ),
            _ => anyhow::bail!("unknown codec: {s}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static NEXT_TEMPORARY_FILE: AtomicUsize = AtomicUsize::new(0);

    // A per-process counter keeps concurrently created fixtures distinct on coarse clocks.
    fn temporary_path(name: &str) -> std::path::PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after the Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ironrdp-perfenc-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ))
    }

    async fn display_updates(name: &str, bytes: &[u8]) -> (DisplayUpdates, std::path::PathBuf) {
        let path = temporary_path(name);
        fs::write(&path, bytes).expect("write temporary frame source");
        let file = File::open(&path).await.expect("open temporary frame source");
        let one = NonZeroU16::new(1).expect("one is nonzero");
        (DisplayUpdates::new(file, one, one, 0), path)
    }

    #[tokio::test]
    async fn returns_none_at_clean_eof() {
        let (mut updates, path) = display_updates("clean-eof", &[]).await;
        assert!(updates.next_update().await.expect("clean EOF is valid").is_none());
        fs::remove_file(path).expect("remove temporary frame source");
    }

    #[tokio::test]
    async fn rejects_truncated_frame() {
        let (mut updates, path) = display_updates("truncated", &[0, 1, 2]).await;
        let error = updates.next_update().await.expect_err("partial frame must fail");
        assert!(error.to_string().contains("truncated RGBX frame"));
        fs::remove_file(path).expect("remove temporary frame source");
    }
}
