#![allow(unused_crate_dependencies)] // False positives because there are both a library and a binary.

use std::{io::Write, str::FromStr};

use anyhow::Context;
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, EntropyBits};
use ironrdp_server::{
    bench::encoder::UpdateEncoder, BitmapUpdate, DesktopSize, DisplayUpdate, PixelFormat, PixelOrder,
    RdpServerDisplayUpdates,
};
use tokio::{fs::File, io::AsyncReadExt};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    setup_logging()?;
    let mut pargs = pico_args::Arguments::from_env();

    let width = pargs.opt_value_from_str("--width")?.unwrap_or(3840);
    let height = pargs.opt_value_from_str("--height")?.unwrap_or(2400);
    let codec = pargs.opt_value_from_str("--codec")?.unwrap_or(OptCodec::default());

    let filename: String = pargs.free_from_str().context("missing RGB input filename")?;
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
        OptCodec::QOI => (None, Some(0)),
    };
    let mut encoder = UpdateEncoder::new(
        flags,
        remotefx,
        #[cfg(feature = "qoi")]
        qoicodec,
    );

    let mut total_raw = 0u64;
    let mut total_enc = 0u64;
    let mut n_updates = 0u64;
    let mut updates = DisplayUpdates::new(file, DesktopSize { width, height });
    while let Some(DisplayUpdate::Bitmap(up)) = updates.next_update().await {
        total_raw += up.data.len() as u64;
        let frag = encoder.bitmap(up)?;
        let len = frag.data.len() as u64;
        total_enc += len;
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
            top: 0,
            left: 0,
            width: self.desktop_size.width.try_into().unwrap(),
            height: self.desktop_size.height.try_into().unwrap(),
            format: PixelFormat::RgbX32,
            order: PixelOrder::TopToBottom,
            data: buf,
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
    #[cfg(feature = "qoi")]
    QOI,
    RemoteFX,
    Bitmap,
    None,
}

impl Default for OptCodec {
    fn default() -> Self {
        #[cfg(feature = "qoi")]
        {
            Self::QOI
        }

        #[cfg(not(feature = "qoi"))]
        {
            Self::RemoteFX
        }
    }
}

impl FromStr for OptCodec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            #[cfg(feature = "qoi")]
            "qoi" => Ok(Self::QOI),
            "remotefx" => Ok(Self::RemoteFX),
            "bitmap" => Ok(Self::Bitmap),
            "none" => Ok(Self::None),
            _ => Err(anyhow::anyhow!("unknown codec: {}", s)),
        }
    }
}
