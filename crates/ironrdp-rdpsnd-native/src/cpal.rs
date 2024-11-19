use std::borrow::Cow;
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use core::time::Duration;

use anyhow::{bail, Context};
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use ironrdp_rdpsnd::client::RdpsndClientHandler;
use ironrdp_rdpsnd::pdu::{AudioFormat, PitchPdu, VolumePdu, WaveFormat};

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
            debug!(?format, "New audio format");
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
                    Err(e) => {
                        error!(error = format!("{e:#}"));
                        return;
                    }
                };
                debug!("Stream thread parking loop");
                while !stream_ended.load(Ordering::Relaxed) {
                    thread::park();
                }
                debug!("Stream thread unparked");
                drop(stream);
            }));
        }

        if let Some(ref tx) = self.tx {
            if let Err(error) = tx.send(data.to_vec()) {
                error!(%error);
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
        bail!("only PCM formats supported");
    }
    let sample_format = match rx_format.bits_per_sample {
        8 => SampleFormat::U8,
        16 => SampleFormat::I16,
        _ => {
            bail!("only PCM 8/16 bits formats supported");
        }
    };

    let host = cpal::default_host();
    let device = host.default_output_device().context("no default output device")?;
    let _supported_configs_range = device
        .supported_output_configs()
        .context("no supported output config")?;
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
            |error| error!(%error),
            None,
        )
        .context("failed to setup output stream")?;

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
                        debug!(rx.len = rx.len());
                        self.last = Some(rx);
                    }
                    Err(error) => {
                        warn!(%error);
                    }
                }
            }

            let Some(ref last) = self.last else {
                info!("Playback rx underrun");
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
