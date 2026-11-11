#![allow(unused_crate_dependencies)] // opus, false negative because it's a separate binary :/
#![expect(clippy::print_stdout, reason = "the example reports to the terminal")]

//! Lists the audio devices, then plays a two-second tone through the RDPSND playback backend.
//!
//! Usage: `cargo run --example cpal -- [DEVICE] [ima|ms|alaw|mulaw|pcm]`
//!
//! The tone is encoded with the chosen codec and fed in 20 ms wave blocks, as a server would send
//! them, and the latency the backend reports for each Wave Confirm is printed.

use core::f64::consts::TAU;
use core::time::Duration;
use std::borrow::Cow;
use std::thread;
use std::time::Instant;

use anyhow::Context as _;
use ironrdp_rdpsnd::client::RdpsndClientHandler as _;
use ironrdp_rdpsnd::codec::{self, AudioEncoder};
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use ironrdp_rdpsnd_native::cpal::{PlaybackConfig, RdpsndBackend};
use ironrdp_rdpsnd_native::{DeviceSelector, Direction, list_devices};

const RATE: u32 = 22050;
const CHANNELS: u16 = 2;

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

fn main() -> anyhow::Result<()> {
    setup_logging()?;
    let mut args = std::env::args().skip(1);
    let device = args
        .next()
        .map_or(DeviceSelector::Default, |device| DeviceSelector::parse(&device));
    let codec = args.next().unwrap_or_else(|| "ima".to_owned());

    for direction in [Direction::Output, Direction::Input] {
        println!("{direction:?} devices:");
        for device in list_devices(direction) {
            let marker = if device.is_default { "*" } else { " " };
            println!(" {marker} {} ({})", device.name, device.id);
        }
    }

    let format = match codec.as_str() {
        "ima" => codec::ima_adpcm_format(CHANNELS, RATE, 1024),
        "ms" => codec::ms_adpcm_format(CHANNELS, RATE, 1024),
        "alaw" => codec::alaw_format(CHANNELS, RATE),
        "mulaw" => codec::mulaw_format(CHANNELS, RATE),
        "pcm" => AudioFormat {
            format: WaveFormat::PCM,
            n_channels: CHANNELS,
            n_samples_per_sec: RATE,
            n_avg_bytes_per_sec: RATE * u32::from(CHANNELS) * 2,
            n_block_align: CHANNELS * 2,
            bits_per_sample: 16,
            data: None,
        },
        other => anyhow::bail!("unknown codec {other}"),
    };

    // A 440 Hz tone on the left channel and 660 Hz on the right.
    let frames = usize::try_from(RATE * 2).context("frame count")?;
    let pcm: Vec<i16> = (0..frames)
        .flat_map(|frame| {
            let time = f64::from(u32::try_from(frame).unwrap_or(0)) / f64::from(RATE);
            [440.0, 660.0].map(|frequency| {
                #[expect(clippy::cast_possible_truncation, clippy::as_conversions, reason = "bounded sine")]
                let sample = ((TAU * frequency * time).sin() * 12_000.0) as i16;
                sample
            })
        })
        .collect();
    let mut encoder = AudioEncoder::new(&format).context("encoder")?;
    let mut wave_data = Vec::new();
    encoder.encode(&pcm, &mut wave_data);

    println!("Playing {codec} on {device}");
    let mut backend = RdpsndBackend::with_config(PlaybackConfig {
        device,
        ..PlaybackConfig::default()
    });
    // Feed 20 ms blocks (whole codec blocks for ADPCM) on a real-time schedule.
    let block_bytes = match format.format {
        WaveFormat::ADPCM | WaveFormat::DVI_ADPCM => usize::from(format.n_block_align),
        _ => usize::try_from(format.n_avg_bytes_per_sec / 50).context("block size")?,
    };
    let block_duration = Duration::from_secs_f64(
        f64::from(u32::try_from(block_bytes).context("block size")?) / f64::from(format.n_avg_bytes_per_sec),
    );
    let start = Instant::now();
    for (index, block) in wave_data.chunks(block_bytes).enumerate() {
        backend.wave(&format, 0, Cow::Borrowed(block));
        if index % 10 == 0 {
            println!("latency after block {index}: {:?}", backend.playback_latency());
        }
        let due = block_duration * u32::try_from(index + 1).context("block index")?;
        thread::sleep(due.saturating_sub(start.elapsed()));
    }
    thread::sleep(Duration::from_millis(300));
    backend.close();

    Ok(())
}
