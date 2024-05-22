use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
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
    // Unfortunately, Stream is not `Send`, so we move it to a separate thread.
    stream_handle: Option<JoinHandle<()>>,
    stream_ended: Arc<AtomicBool>,
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
            stream_ended: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Drop for RdpsndBackend {
    fn drop(&mut self) {
        self.close();
    }
}

impl RdpsndClientHandler for RdpsndBackend {
    fn wave(&mut self, format: &AudioFormat, _ts: u32, data: Cow<'_, [u8]>) {
        if Some(format) != self.format.as_ref() {
            debug!("new audio format {format:?}");
            self.close();
        }

        if self.stream_handle.is_none() {
            let (tx, rx) = mpsc::channel();
            self.tx = Some(tx);
            self.format = Some(format.clone());
            let format = format.clone();
            self.stream_ended.store(false, Ordering::Relaxed);
            let stream_ended = Arc::clone(&self.stream_ended);
            self.stream_handle = Some(thread::spawn(move || {
                let stream = match make_stream(&format, rx) {
                    Ok(stream) => stream,
                    Err(err) => {
                        error!("{}", err);
                        return;
                    }
                };
                debug!("stream thread parking loop");
                while !stream_ended.load(Ordering::Relaxed) {
                    thread::park();
                }
                debug!("stream thread unparked");
                drop(stream);
            }));
        }

        if let Some(ref tx) = self.tx {
            if let Err(err) = tx.send(data.to_vec()) {
                error!("{}", err);
            }
        };
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        debug!(?volume);
    }

    fn set_pitch(&mut self, pitch: PitchPdu) {
        debug!(?pitch);
    }

    fn close(&mut self) {
        self.tx = None;
        if let Some(stream) = self.stream_handle.take() {
            self.stream_ended.store(true, Ordering::Relaxed);
            stream.thread().unpark();
            stream.join().unwrap();
        }
    }
}

#[doc(hidden)]
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
                        info!(?err);
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
