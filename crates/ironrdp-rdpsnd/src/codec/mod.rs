//! Wave data codecs for audio output ([MS-RDPEA]) and audio input ([MS-RDPEAI]).
//!
//! [`AudioDecoder`] turns the wave data of a negotiated [`AudioFormat`] into interleaved signed
//! 16-bit PCM, and [`AudioEncoder`] does the reverse. Both are pure computation: they hold the
//! codec state between calls and perform no I/O.
//!
//! Implemented formats: 8-bit and 16-bit PCM, G.711 A-law and µ-law, IMA (DVI) ADPCM and
//! Microsoft ADPCM. These are the codecs Windows ships for audio redirection besides AAC, GSM 6.10
//! and G.723.1 ([MS-RDPEA] Appendix A, "Section 2.2.2.1.1").
//!
//! [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
//! [MS-RDPEAI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeai/2eb8be0c-4f17-418b-9911-edb8d2ffcde5

mod adpcm;
mod g711;

use core::fmt;

pub use g711::{alaw_to_linear, linear_to_alaw, linear_to_ulaw, ulaw_to_linear};

use crate::pdu::{AudioFormat, WaveFormat};

/// Failure to build a codec for a format or to decode its wave data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodecError {
    /// The wave format tag has no codec in this module.
    UnsupportedFormat(WaveFormat),
    /// The format parameters are outside what the codec supports.
    InvalidFormat(&'static str),
    /// The wave data cannot be decoded with the format.
    InvalidData(&'static str),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat(format) => write!(f, "no codec for wave format {format}"),
            Self::InvalidFormat(reason) => write!(f, "invalid audio format: {reason}"),
            Self::InvalidData(reason) => write!(f, "invalid wave data: {reason}"),
        }
    }
}

impl core::error::Error for CodecError {}

#[derive(Debug, Clone)]
enum Codec {
    Pcm8,
    Pcm16,
    ALaw,
    MuLaw,
    ImaAdpcm(adpcm::ImaAdpcm),
    MsAdpcm(adpcm::MsAdpcm),
}

impl Codec {
    fn new(format: &AudioFormat) -> Result<Self, CodecError> {
        if format.n_channels == 0 {
            return Err(CodecError::InvalidFormat("zero channels"));
        }
        if format.n_samples_per_sec == 0 {
            return Err(CodecError::InvalidFormat("zero sample rate"));
        }
        let channels = usize::from(format.n_channels);
        let block_align = usize::from(format.n_block_align);

        match format.format {
            WaveFormat::PCM => match format.bits_per_sample {
                8 => Ok(Self::Pcm8),
                16 => Ok(Self::Pcm16),
                _ => Err(CodecError::InvalidFormat("PCM supports 8-bit and 16-bit samples")),
            },
            WaveFormat::ALAW | WaveFormat::MULAW => {
                if format.bits_per_sample != 8 {
                    return Err(CodecError::InvalidFormat("G.711 uses 8-bit samples"));
                }
                Ok(if format.format == WaveFormat::ALAW {
                    Self::ALaw
                } else {
                    Self::MuLaw
                })
            }
            WaveFormat::DVI_ADPCM => {
                if format.bits_per_sample != 4 {
                    return Err(CodecError::InvalidFormat("IMA ADPCM uses 4-bit samples"));
                }
                Ok(Self::ImaAdpcm(adpcm::ImaAdpcm::new(channels, block_align)?))
            }
            WaveFormat::ADPCM => {
                if format.bits_per_sample != 4 {
                    return Err(CodecError::InvalidFormat("Microsoft ADPCM uses 4-bit samples"));
                }
                let (frames_per_block, coefficients) = parse_ms_adpcm_extra(format.data.as_deref())?;
                Ok(Self::MsAdpcm(adpcm::MsAdpcm::new(
                    channels,
                    block_align,
                    frames_per_block,
                    coefficients,
                )?))
            }
            other => Err(CodecError::UnsupportedFormat(other)),
        }
    }
}

/// Decoder from negotiated wave data to interleaved signed 16-bit PCM.
#[derive(Debug, Clone)]
pub struct AudioDecoder {
    codec: Codec,
}

impl AudioDecoder {
    /// Builds a decoder for `format`.
    ///
    /// The decoded PCM has the channel count and sample rate of `format`.
    pub fn new(format: &AudioFormat) -> Result<Self, CodecError> {
        Ok(Self {
            codec: Codec::new(format)?,
        })
    }

    /// Returns whether a decoder can be built for `format`.
    pub fn supports(format: &AudioFormat) -> bool {
        Codec::new(format).is_ok()
    }

    /// Decodes `data` and appends the interleaved samples to `out`.
    ///
    /// ADPCM data is split into blocks of `nBlockAlign` bytes; a shorter final block is decoded as
    /// far as it goes. On error, `out` keeps the samples of the blocks decoded before the failure.
    ///
    /// Each call must start on a block boundary: no state carries over between calls, because a
    /// wave sample is a self-contained piece of encoded audio. Windows servers encode every sample
    /// with the ACM codec, which emits whole blocks, so wave PDUs never split a block.
    pub fn decode(&mut self, data: &[u8], out: &mut Vec<i16>) -> Result<(), CodecError> {
        match &mut self.codec {
            Codec::Pcm8 => out.extend(data.iter().map(|sample| (i16::from(*sample) - 128) << 8)),
            Codec::Pcm16 => out.extend(
                data.chunks_exact(2)
                    .map(|sample| i16::from_le_bytes([sample[0], sample[1]])),
            ),
            Codec::ALaw => out.extend(data.iter().copied().map(alaw_to_linear)),
            Codec::MuLaw => out.extend(data.iter().copied().map(ulaw_to_linear)),
            Codec::ImaAdpcm(codec) => {
                for block in data.chunks(codec.block_align()) {
                    codec.decode_block(block, out)?;
                }
            }
            Codec::MsAdpcm(codec) => {
                for block in data.chunks(codec.block_align()) {
                    codec.decode_block(block, out)?;
                }
            }
        }
        Ok(())
    }
}

/// Encoder from interleaved signed 16-bit PCM to wave data.
///
/// Block codecs (the ADPCM variants) buffer input until a whole block is available.
#[derive(Debug, Clone)]
pub struct AudioEncoder {
    codec: Codec,
    channels: usize,
    pending: Vec<i16>,
}

impl AudioEncoder {
    /// Builds an encoder for `format`.
    pub fn new(format: &AudioFormat) -> Result<Self, CodecError> {
        Ok(Self {
            codec: Codec::new(format)?,
            channels: usize::from(format.n_channels),
            pending: Vec::new(),
        })
    }

    /// Returns whether an encoder can be built for `format`.
    pub fn supports(format: &AudioFormat) -> bool {
        Codec::new(format).is_ok()
    }

    /// Frames consumed by one encoded block (one for sample-based codecs).
    pub fn frames_per_block(&self) -> usize {
        match &self.codec {
            Codec::Pcm8 | Codec::Pcm16 | Codec::ALaw | Codec::MuLaw => 1,
            Codec::ImaAdpcm(codec) => codec.frames_per_block(),
            Codec::MsAdpcm(codec) => codec.frames_per_block(),
        }
    }

    /// Encodes interleaved samples and appends every completed block to `out`.
    ///
    /// Samples that do not complete a block stay buffered for the next call.
    pub fn encode(&mut self, pcm: &[i16], out: &mut Vec<u8>) {
        match &mut self.codec {
            Codec::Pcm8 => out.extend(pcm.iter().map(|sample| (sample.to_le_bytes()[1]) ^ 0x80)),
            Codec::Pcm16 => out.extend(pcm.iter().flat_map(|sample| sample.to_le_bytes())),
            Codec::ALaw => out.extend(pcm.iter().copied().map(linear_to_alaw)),
            Codec::MuLaw => out.extend(pcm.iter().copied().map(linear_to_ulaw)),
            Codec::ImaAdpcm(codec) => {
                let block = codec.frames_per_block() * self.channels;
                encode_blocks(&mut self.pending, pcm, block, |frames| codec.encode_block(frames, out));
            }
            Codec::MsAdpcm(codec) => {
                let block = codec.frames_per_block() * self.channels;
                encode_blocks(&mut self.pending, pcm, block, |frames| codec.encode_block(frames, out));
            }
        }
    }
}

fn encode_blocks(pending: &mut Vec<i16>, pcm: &[i16], block: usize, mut encode: impl FnMut(&[i16])) {
    let mut input = pcm;
    if !pending.is_empty() {
        let missing = block - pending.len();
        if input.len() < missing {
            pending.extend_from_slice(input);
            return;
        }
        let (head, rest) = input.split_at(missing);
        pending.extend_from_slice(head);
        encode(pending);
        pending.clear();
        input = rest;
    }
    let mut blocks = input.chunks_exact(block);
    for frames in &mut blocks {
        encode(frames);
    }
    pending.extend_from_slice(blocks.remainder());
}

/// Samples per block (when given) and predictor coefficients of a Microsoft ADPCM format.
type MsAdpcmParameters = (Option<usize>, Vec<(i16, i16)>);

/// Parses the `ADPCMWAVEFORMAT` extra bytes: samples per block and predictor coefficients.
fn parse_ms_adpcm_extra(data: Option<&[u8]>) -> Result<MsAdpcmParameters, CodecError> {
    let Some(data) = data else {
        return Ok((None, adpcm::MS_STANDARD_COEFFICIENTS.to_vec()));
    };
    let Some((header, coefficients)) = data.split_at_checked(2 /* wSamplesPerBlock */ + 2 /* wNumCoef */) else {
        return Err(CodecError::InvalidFormat("Microsoft ADPCM extra data is too short"));
    };
    let frames_per_block = usize::from(u16::from_le_bytes([header[0], header[1]]));
    let count = usize::from(u16::from_le_bytes([header[2], header[3]]));
    let Some(coefficients) = coefficients.get(..count * (2 /* iCoef1 */ + 2/* iCoef2 */)) else {
        return Err(CodecError::InvalidFormat(
            "Microsoft ADPCM coefficient table is truncated",
        ));
    };
    let coefficients = coefficients
        .chunks_exact(4)
        .map(|pair| {
            (
                i16::from_le_bytes([pair[0], pair[1]]),
                i16::from_le_bytes([pair[2], pair[3]]),
            )
        })
        .collect();
    Ok((Some(frames_per_block), coefficients))
}

/// Builds a G.711 A-law format.
pub fn alaw_format(channels: u16, sample_rate: u32) -> AudioFormat {
    g711_format(WaveFormat::ALAW, channels, sample_rate)
}

/// Builds a G.711 µ-law format.
pub fn mulaw_format(channels: u16, sample_rate: u32) -> AudioFormat {
    g711_format(WaveFormat::MULAW, channels, sample_rate)
}

fn g711_format(format: WaveFormat, channels: u16, sample_rate: u32) -> AudioFormat {
    AudioFormat {
        format,
        n_channels: channels,
        n_samples_per_sec: sample_rate,
        n_avg_bytes_per_sec: sample_rate * u32::from(channels),
        n_block_align: channels,
        bits_per_sample: 8,
        data: None,
    }
}

/// Builds an IMA ADPCM format with `block_align`-byte blocks, laid out as the Windows codec does.
pub fn ima_adpcm_format(channels: u16, sample_rate: u32, block_align: u16) -> AudioFormat {
    let frames = adpcm::ima_frames_per_block(usize::from(channels), usize::from(block_align));
    let frames = u16::try_from(frames).unwrap_or(u16::MAX);
    AudioFormat {
        format: WaveFormat::DVI_ADPCM,
        n_channels: channels,
        n_samples_per_sec: sample_rate,
        n_avg_bytes_per_sec: average_bytes_per_second(sample_rate, block_align, frames),
        n_block_align: block_align,
        bits_per_sample: 4,
        data: Some(frames.to_le_bytes().to_vec()),
    }
}

/// Builds a Microsoft ADPCM format with `block_align`-byte blocks and the standard coefficients.
pub fn ms_adpcm_format(channels: u16, sample_rate: u32, block_align: u16) -> AudioFormat {
    let frames = adpcm::ms_frames_per_block(usize::from(channels), usize::from(block_align));
    let frames = u16::try_from(frames).unwrap_or(u16::MAX);
    let count = u16::try_from(adpcm::MS_STANDARD_COEFFICIENTS.len()).unwrap_or(u16::MAX);
    let mut data = Vec::with_capacity(4 + 4 * adpcm::MS_STANDARD_COEFFICIENTS.len());
    data.extend_from_slice(&frames.to_le_bytes());
    data.extend_from_slice(&count.to_le_bytes());
    for (coefficient1, coefficient2) in adpcm::MS_STANDARD_COEFFICIENTS {
        data.extend_from_slice(&coefficient1.to_le_bytes());
        data.extend_from_slice(&coefficient2.to_le_bytes());
    }
    AudioFormat {
        format: WaveFormat::ADPCM,
        n_channels: channels,
        n_samples_per_sec: sample_rate,
        n_avg_bytes_per_sec: average_bytes_per_second(sample_rate, block_align, frames),
        n_block_align: block_align,
        bits_per_sample: 4,
        data: Some(data),
    }
}

fn average_bytes_per_second(sample_rate: u32, block_align: u16, frames_per_block: u16) -> u32 {
    let bytes = u64::from(sample_rate) * u64::from(block_align) / u64::from(frames_per_block.max(1));
    u32::try_from(bytes).unwrap_or(u32::MAX)
}
