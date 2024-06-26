use std::{
    borrow::Cow,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{bail, Context};
use cpal::{
    traits::{DeviceTrait, HostTrait},
    SampleFormat, Stream, StreamConfig,
};
use ironrdp_rdpsnd::{
    client::RdpsndClientHandler,
    pdu::{AudioFormat, PitchPdu, VolumePdu, WaveFormat},
};
use tracing::{debug, error, info};

#[derive(Debug)]
pub struct RdpsndBackend {
    stream_handle: Option<JoinHandle<()>>,
    tx: Option<Sender<Vec<u8>>>,
    format: Option<AudioFormat>,
}

impl Default for RdpsndBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpsndBackend {
    pub fn new() -> Self {
        Self {
            tx: None,
            format: None,
            stream_handle: None,
        }
    }
}

impl RdpsndClientHandler for RdpsndBackend {
    fn wave(&mut self, format: &AudioFormat, _ts: u32, data: Cow<'_, [u8]>) {
        if Some(format) != self.format.as_ref() {
            debug!("new audio format {format:?}");
            if let Some(ref stream) = self.stream_handle {
                stream.thread().unpark();
                self.stream_handle = None;
            }
        }

        if self.stream_handle.is_none() {
            let (tx, rx) = mpsc::channel();
            self.tx = Some(tx);
            self.format = Some(format.clone());
            let format = format.clone();
            self.stream_handle = Some(thread::spawn(move || {
                let _stream = match make_stream(&format, rx) {
                    Ok(stream) => Some(stream),
                    Err(err) => {
                        error!("{}", err);
                        return;
                    }
                };
                debug!("parking");
                thread::park();
                debug!("unparking");
            }));
        }

        if let Some(ref tx) = self.tx {
            if let Err(err) = tx.send(data.to_vec()) {
                error!("{}", err);
            }
        };
    }

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {
        self.tx = None;
    }
}

pub fn make_stream(rx_format: &AudioFormat, rx: Receiver<Vec<u8>>) -> anyhow::Result<Stream> {
    if rx_format.format != WaveFormat::PCM {
        bail!("Only PCM formats supported");
    }
    let sample_format = match rx_format.bits_per_sample {
        8 => SampleFormat::U8,
        16 => SampleFormat::I16,
        _ => {
            bail!("Only PCM 8/16 bits formats supported");
        }
    };

    let host = cpal::default_host();
    let device = host.default_output_device().context("No default output device")?;
    let _supported_configs_range = device
        .supported_output_configs()
        .context("No supported output config")?;
    let default_config = device.default_output_config()?;
    debug!(?default_config);

    let mut rx = RxBuffer::new(rx);
    let config = StreamConfig {
        channels: rx_format.n_channels,
        sample_rate: cpal::SampleRate(rx_format.n_samples_per_sec),
        buffer_size: cpal::BufferSize::Default,
    };
    debug!(?config);

    let stream = device
        .build_output_stream_raw(
            &config,
            sample_format,
            move |data, _info: &cpal::OutputCallbackInfo| {
                let data = data.bytes_mut();
                rx.fill(data)
            },
            |err| error!(?err),
            None,
        )
        .context("Failed to setup output stream")?;

    Ok(stream)
}

struct RxBuffer {
    receiver: Receiver<Vec<u8>>,
    last: Option<Vec<u8>>,
    idx: usize,
}

impl RxBuffer {
    fn new(receiver: Receiver<Vec<u8>>) -> Self {
        Self {
            receiver,
            last: None,
            idx: 0,
        }
    }

    fn fill(&mut self, data: &mut [u8]) {
        let mut filled = 0;

        while filled < data.len() {
            if self.last.is_none() {
                match self.receiver.recv_timeout(Duration::from_millis(4000)) {
                    Ok(rx) => {
                        debug!("{}", rx.len());
                        self.last = Some(rx);
                    }
                    Err(err) => {
                        error!(?err);
                    }
                }
            }

            let Some(ref last) = self.last else {
                info!("playback rx underrun");
                return;
            };

            #[allow(clippy::arithmetic_side_effects)]
            while self.idx < last.len() && filled < data.len() {
                data[filled] = last[self.idx];
                assert!(filled < usize::MAX);
                assert!(self.idx < usize::MAX);
                filled += 1;
                self.idx += 1;
            }

            // If all elements from last have been consumed, clear `self.last`
            if self.idx >= last.len() {
                self.last = None;
                self.idx = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread, time::Duration};

    use cpal::traits::StreamTrait;
    use ironrdp_rdpsnd::pdu::WaveFormat;

    use super::*;

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

    #[test]
    fn it_works() {
        setup_logging().unwrap();

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

        let _producer = thread::spawn(move || {
            let data_chunks = vec![vec![1u8, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
            for chunk in data_chunks {
                tx.send(chunk).expect("Failed to send data chunk");
                debug!("sent a chunk");
                thread::sleep(Duration::from_secs(1)); // Simulating work
            }
        });

        stream.play().unwrap();
        std::thread::sleep(Duration::from_millis(4000));
    }
}
