#![allow(unused_crate_dependencies)] // False positives because there are both a library and a binary.
#![allow(clippy::print_stderr)]
#![allow(clippy::print_stdout)]

use std::io::Write;

use anyhow::Context;
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, EntropyBits};
use ironrdp_server::{
    bench::encoder::UpdateEncoder, BitmapUpdate, DesktopSize, DisplayUpdate, PixelFormat, RdpServerDisplayUpdates,
};
use tokio::{fs::File, io::AsyncReadExt};

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
        println!("  --codec <CODEC>      Codec to use (default: RemoteFX)");
        println!("                        Valid values: RemoteFX, Bitmap, None");
        std::process::exit(0);
    }

    let width = args.opt_value_from_str("--width")?.unwrap_or(3840);
    let height = args.opt_value_from_str("--height")?.unwrap_or(2400);
    let codec = args.opt_value_from_str("--codec")?.unwrap_or_else(OptCodec::default);

    let filename: String = args.free_from_str().context("missing RGBX input filename")?;
    let file = File::open(&filename)
        .await
        .with_context(|| format!("Failed to open file: {}", filename))?;

    let mut flags = CmdFlags::all();

    #[allow(unused)]
    let (remotefx, qoicodec) = match codec {
        OptCodec::RemoteFX => (Some((EntropyBits::Rlgr3, 0)), None::<u8>),
        OptCodec::Bitmap => {
            flags -= CmdFlags::SET_SURFACE_BITS;
            (None, None)
        }
        OptCodec::None => (None, None),
        #[cfg(feature = "qoi")]
        OptCodec::Qoi => (None, Some(0)),
    };
    let mut encoder = UpdateEncoder::new(
        DesktopSize { width, height },
        flags,
        remotefx,
        #[cfg(feature = "qoi")]
        qoicodec,
    );

    let mut total_raw = 0u64;
    let mut total_enc = 0u64;
    let mut n_updates = 0u64;
    let mut updates = DisplayUpdates::new(file, DesktopSize { width, height });
    while let Some(up) = updates.next_update().await {
        if let DisplayUpdate::Bitmap(ref up) = up {
            total_raw += up.data.len() as u64;
        } else {
            eprintln!("Invalid update");
            break;
        }
        let mut iter = encoder.update(up);
        loop {
            let Some(frag) = iter.next().await else {
                break;
            };
            let len = frag?.data.len() as u64;
            total_enc += len;
        }
        n_updates += 1;
        print!(".");
        std::io::stdout().flush().unwrap();
    }
    println!();

    let ratio = total_enc as f64 / total_raw as f64;
    let percent = 100.0 - ratio * 100.0;
    println!("Encoder: {:?}", encoder);
    println!("Nb updates: {:?}", n_updates);
    println!(
        "Sum of bytes: {}/{} ({:.2}%)",
        bytesize::ByteSize(total_enc),
        bytesize::ByteSize(total_raw),
        percent,
    );
    Ok(())
}

struct DisplayUpdates {
    file: File,
    desktop_size: DesktopSize,
}

impl DisplayUpdates {
    fn new(file: File, desktop_size: DesktopSize) -> Self {
        Self { file, desktop_size }
    }
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for DisplayUpdates {
    async fn next_update(&mut self) -> Option<DisplayUpdate> {
        let stride = self.desktop_size.width as usize * 4;
        let frame_size = stride * self.desktop_size.height as usize;
        let mut buf = vec![0u8; frame_size];
        if self.file.read_exact(&mut buf).await.is_err() {
            return None;
        }

        let up = DisplayUpdate::Bitmap(BitmapUpdate {
            x: 0,
            y: 0,
            width: self.desktop_size.width.try_into().unwrap(),
            height: self.desktop_size.height.try_into().unwrap(),
            format: PixelFormat::RgbX32,
            data: buf.into(),
            stride,
        });
        Some(up)
    }
}

fn setup_logging() -> anyhow::Result<()> {
    use tracing::metadata::LevelFilter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

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

enum OptCodec {
    RemoteFX,
    Bitmap,
    None,
    #[cfg(feature = "qoi")]
    Qoi,
}

impl Default for OptCodec {
    fn default() -> Self {
        Self::RemoteFX
    }
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
            _ => Err(anyhow::anyhow!("unknown codec: {}", s)),
        }
    }
}
