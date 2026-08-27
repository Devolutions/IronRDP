#![allow(unused_crate_dependencies)] // opus, false negative because it's a separate binary :/

use core::sync::atomic::AtomicU32;
use core::time::Duration;
use std::sync::Arc;
use std::thread;

use anyhow::Context as _;
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use ironrdp_rdpsnd_native::cpal::DecodeStream;
use tracing::debug;

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
    let rx_format = AudioFormat {
        format: WaveFormat::PCM,
        n_channels: 2,
        n_samples_per_sec: 22050,
        n_avg_bytes_per_sec: 88200,
        n_block_align: 4,
        bits_per_sample: 16,
        data: None,
    };
    // Full volume on both channels (internal pack_volume layout: left high, right low).
    let volume = Arc::new(AtomicU32::new(0xFFFF_FFFF));
    let (_stream, mut producer) = DecodeStream::new(&rx_format, volume)?;

    let producer_thread = thread::spawn(move || {
        let data_chunks = vec![vec![1u8, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        for chunk in data_chunks {
            let written = producer.push_slice(&chunk);
            debug_assert_eq!(written, chunk.len(), "ring buffer too small for this example chunk");
            debug!("Sent a chunk");
            thread::sleep(Duration::from_secs(1)); // Simulating work
        }
    });

    thread::sleep(Duration::from_secs(3));
    let _ = producer_thread.join();

    Ok(())
}
