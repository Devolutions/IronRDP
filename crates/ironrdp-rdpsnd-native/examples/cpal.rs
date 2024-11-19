use core::time::Duration;
use std::sync::mpsc;
use std::thread;

use anyhow::Context;
use cpal::traits::StreamTrait;
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use ironrdp_rdpsnd_native::cpal::make_stream;
use tracing::debug;

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

fn main() -> anyhow::Result<()> {
    setup_logging()?;
    let rx_format = AudioFormat {
        format: WaveFormat::PCM,
        n_channels: 2,
        n_samples_per_sec: 22050,
        n_avg_bytes_per_sec: 88200,
        n_block_align: 4,
        bits_per_sample: 16,
        data: None,
    };
    let (tx, rx) = mpsc::channel();
    let stream = make_stream(&rx_format, rx).unwrap();

    let producer = thread::spawn(move || {
        let data_chunks = vec![vec![1u8, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        for chunk in data_chunks {
            tx.send(chunk).expect("failed to send data chunk");
            debug!("Sent a chunk");
            thread::sleep(Duration::from_secs(1)); // Simulating work
        }
    });

    stream.play()?;
    thread::sleep(Duration::from_secs(3));
    let _ = producer.join();

    Ok(())
}
