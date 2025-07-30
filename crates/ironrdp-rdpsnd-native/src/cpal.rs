use core::mem::size_of;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::borrow::Cow;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use anyhow::{bail, Context as _};
use cpal::traits::{DeviceTrait as _, HostTrait as _};
use cpal::{SampleFormat, Stream, StreamConfig};
use ironrdp_rdpsnd::client::RdpsndClientHandler;
use ironrdp_rdpsnd::pdu::{AudioFormat, PitchPdu, VolumePdu, WaveFormat};

#[derive(Debug)]
pub struct RdpsndBackend {
    // Unfortunately, Stream is not `Send`, so we move it to a separate thread.
    stream_handle: Option<JoinHandle<()>>,
    stream_ended: Arc<AtomicBool>,
    tx: Option<Sender<Vec<u8>>>,
    format_no: Option<usize>,
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
            format_no: None,
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
    fn get_formats(&self) -> &[AudioFormat] {
        &[
            #[cfg(feature = "opus")]
            AudioFormat {
                format: WaveFormat::OPUS,
                n_channels: 2,
                n_samples_per_sec: 48000,
                n_avg_bytes_per_sec: 192000,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
            AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: 44100,
                n_avg_bytes_per_sec: 176400,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
        ]
    }

    fn wave(&mut self, format_no: usize, _ts: u32, data: Cow<'_, [u8]>) {
        if Some(format_no) != self.format_no {
            debug!("New audio format");
            self.close();
        }

        if self.stream_handle.is_none() {
            let (tx, rx) = mpsc::channel();
            self.tx = Some(tx);

            self.format_no = Some(format_no);
            let Some(format) = self.get_formats().get(format_no) else {
                warn!(?format_no, "Invalid format_no");
                return;
            };
            let format = format.clone();
            self.stream_ended.store(false, Ordering::Relaxed);
            let stream_ended = Arc::clone(&self.stream_ended);
            self.stream_handle = Some(thread::spawn(move || {
                let stream = match DecodeStream::new(&format, rx) {
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
pub struct DecodeStream {
    _dec_thread: Option<JoinHandle<()>>,
    pub stream: Stream,
}

impl DecodeStream {
    pub fn new(rx_format: &AudioFormat, mut rx: Receiver<Vec<u8>>) -> anyhow::Result<Self> {
        let mut dec_thread = None;
        match rx_format.format {
            #[cfg(feature = "opus")]
            WaveFormat::OPUS => {
                let chan = match rx_format.n_channels {
                    1 => opus::Channels::Mono,
                    2 => opus::Channels::Stereo,
                    _ => bail!("unsupported #channels for Opus"),
                };
                let (dec_tx, dec_rx) = mpsc::channel();
                let mut dec = opus::Decoder::new(rx_format.n_samples_per_sec, chan)?;
                dec_thread = Some(thread::spawn(move || {
                    while let Ok(pkt) = rx.recv() {
                        let nb_samples = dec.get_nb_samples(&pkt).unwrap();
                        let mut pcm = vec![0u8; nb_samples * chan as usize * size_of::<i16>()];
                        dec.decode(&pkt, bytemuck::cast_slice_mut(pcm.as_mut_slice()), false)
                            .unwrap();
                        dec_tx.send(pcm).unwrap();
                    }
                }));
                rx = dec_rx;
            }
            WaveFormat::PCM => {}
            _ => bail!("audio format not supported"),
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

        Ok(Self {
            _dec_thread: dec_thread,
            stream,
        })
    }
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

            #[expect(clippy::arithmetic_side_effects)]
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
