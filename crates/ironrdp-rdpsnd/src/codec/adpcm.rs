//! IMA (DVI) ADPCM and Microsoft ADPCM block codecs.
//!
//! Both follow the block layouts of the Windows ACM codecs ("IMA ADPCM" and "Microsoft ADPCM"):
//! every block starts with a per-channel header that seeds the predictor, followed by 4-bit codes.

use super::CodecError;

/// IMA ADPCM quantizer step sizes.
const IMA_STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107,
    118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963,
    1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894,
    6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];

/// IMA ADPCM step index adjustment, indexed by the code magnitude (the low three bits).
const IMA_INDEX_TABLE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

const IMA_MAX_STEP_INDEX: i32 = 88;

/// Microsoft ADPCM delta adaptation factors, indexed by the 4-bit code.
const MS_ADAPTATION_TABLE: [i32; 16] = [
    230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
];

/// The seven predictor coefficient pairs every Microsoft ADPCM encoder writes.
pub(super) const MS_STANDARD_COEFFICIENTS: [(i16, i16); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];

const MS_MIN_DELTA: i32 = 16;

/// Upper bound on the adaptive delta, so `768 * delta` cannot overflow.
const MS_MAX_DELTA: i32 = i32::MAX / 768;

/// Size of one channel's IMA ADPCM block header: predictor sample, step index, reserved byte.
pub(super) const IMA_HEADER_SIZE: usize = 2 /* iSamp0 */ + 1 /* bStepTableIndex */ + 1 /* bReserved */;

/// Size of one channel's Microsoft ADPCM block header.
pub(super) const MS_HEADER_SIZE: usize = 1 /* bPredictor */ + 2 /* iDelta */ + 2 /* iSamp1 */ + 2 /* iSamp2 */;

/// Frames carried by an IMA ADPCM block of `block_align` bytes.
///
/// The header sample is the first frame; each further group of four bytes per channel holds eight
/// frames.
pub(super) fn ima_frames_per_block(channels: usize, block_align: usize) -> usize {
    1 + (block_align - IMA_HEADER_SIZE * channels) * 2 / channels
}

/// Frames carried by a Microsoft ADPCM block of `block_align` bytes.
///
/// The two header samples are the first two frames; each further byte holds two codes.
pub(super) fn ms_frames_per_block(channels: usize, block_align: usize) -> usize {
    2 + (block_align - MS_HEADER_SIZE * channels) * 2 / channels
}

#[derive(Debug, Clone, Copy, Default)]
struct ImaChannel {
    predictor: i32,
    step_index: i32,
}

impl ImaChannel {
    fn expand(&mut self, code: u8) -> i16 {
        let step = IMA_STEP_TABLE[step_slot(self.step_index)];
        let mut diff = step >> 3;
        if code & 4 != 0 {
            diff += step;
        }
        if code & 2 != 0 {
            diff += step >> 1;
        }
        if code & 1 != 0 {
            diff += step >> 2;
        }
        if code & 8 != 0 {
            self.predictor -= diff;
        } else {
            self.predictor += diff;
        }
        self.predictor = self.predictor.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        self.step_index = (self.step_index + IMA_INDEX_TABLE[usize::from(code & 7)]).clamp(0, IMA_MAX_STEP_INDEX);
        clamp_i16(self.predictor)
    }

    fn compress(&mut self, sample: i16) -> u8 {
        let mut step = IMA_STEP_TABLE[step_slot(self.step_index)];
        let mut diff = i32::from(sample) - self.predictor;
        let mut code = 0;
        if diff < 0 {
            code = 8;
            diff = -diff;
        }
        for bit in [4, 2, 1] {
            if step <= diff {
                code |= bit;
                diff -= step;
            }
            step >>= 1;
        }
        // Track the value the decoder reconstructs, not the input, so both sides stay in step.
        self.expand(code);
        code
    }
}

fn step_slot(step_index: i32) -> usize {
    // INVARIANT: step_index is kept within 0..=IMA_MAX_STEP_INDEX.
    usize::try_from(step_index).expect("IMA step index is non-negative")
}

/// IMA ADPCM block decoder and encoder state.
#[derive(Debug, Clone)]
pub(super) struct ImaAdpcm {
    channels: usize,
    block_align: usize,
    state: Vec<ImaChannel>,
}

impl ImaAdpcm {
    pub(super) fn new(channels: usize, block_align: usize) -> Result<Self, CodecError> {
        if !(1..=2).contains(&channels) {
            return Err(CodecError::InvalidFormat("IMA ADPCM supports one or two channels"));
        }
        let header = IMA_HEADER_SIZE * channels;
        let group = 4 * channels;
        if block_align <= header || !(block_align - header).is_multiple_of(group) {
            return Err(CodecError::InvalidFormat(
                "IMA ADPCM block alignment does not hold whole code groups",
            ));
        }
        Ok(Self {
            channels,
            block_align,
            state: vec![ImaChannel::default(); channels],
        })
    }

    pub(super) fn block_align(&self) -> usize {
        self.block_align
    }

    pub(super) fn frames_per_block(&self) -> usize {
        ima_frames_per_block(self.channels, self.block_align)
    }

    /// Decodes one block, which may be a truncated final block.
    pub(super) fn decode_block(&mut self, block: &[u8], out: &mut Vec<i16>) -> Result<(), CodecError> {
        let channels = self.channels;
        let header_size = IMA_HEADER_SIZE * channels;
        let Some((header, codes)) = block.split_at_checked(header_size) else {
            return Err(CodecError::InvalidData("IMA ADPCM block is shorter than its header"));
        };

        for (channel, header) in self.state.iter_mut().zip(header.chunks_exact(IMA_HEADER_SIZE)) {
            channel.predictor = i32::from(i16::from_le_bytes([header[0], header[1]]));
            channel.step_index = i32::from(header[2]).min(IMA_MAX_STEP_INDEX);
            out.push(clamp_i16(channel.predictor));
        }

        // Each group holds four bytes (eight frames) for every channel in turn.
        for group in codes.chunks_exact(4 * channels) {
            let base = out.len();
            out.resize(base + 8 * channels, 0);
            for (index, (channel, bytes)) in self.state.iter_mut().zip(group.chunks_exact(4)).enumerate() {
                for (pair, byte) in bytes.iter().enumerate() {
                    out[base + 2 * pair * channels + index] = channel.expand(byte & 0x0F);
                    out[base + (2 * pair + 1) * channels + index] = channel.expand(byte >> 4);
                }
            }
        }

        Ok(())
    }

    /// Encodes exactly one block of `frames_per_block()` interleaved frames.
    pub(super) fn encode_block(&mut self, frames: &[i16], out: &mut Vec<u8>) {
        let channels = self.channels;
        debug_assert_eq!(frames.len(), self.frames_per_block() * channels);

        for (channel, sample) in self.state.iter_mut().zip(frames) {
            channel.predictor = i32::from(*sample);
            out.extend_from_slice(&sample.to_le_bytes());
            out.push(u8::try_from(channel.step_index).expect("step index fits in u8"));
            out.push(0);
        }

        for group in frames[channels..].chunks_exact(8 * channels) {
            for (index, channel) in self.state.iter_mut().enumerate() {
                for pair in 0..4 {
                    let low = channel.compress(group[2 * pair * channels + index]);
                    let high = channel.compress(group[(2 * pair + 1) * channels + index]);
                    out.push(low | (high << 4));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct MsChannel {
    coefficient1: i32,
    coefficient2: i32,
    delta: i32,
    sample1: i32,
    sample2: i32,
}

impl MsChannel {
    fn prediction(&self) -> i32 {
        let sum = i64::from(self.sample1) * i64::from(self.coefficient1)
            + i64::from(self.sample2) * i64::from(self.coefficient2);
        // The coefficient base is 256 and the division truncates toward zero, as in the ACM codec.
        let prediction = sum / 256;
        i32::try_from(prediction.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).expect("clamped to i32")
    }

    fn update(&mut self, code: u8, signed_code: i32) -> i16 {
        let reconstructed = i64::from(self.prediction()) + i64::from(signed_code) * i64::from(self.delta);
        let sample = clamp_i16_wide(reconstructed);
        self.sample2 = self.sample1;
        self.sample1 = i32::from(sample);
        self.delta = ((MS_ADAPTATION_TABLE[usize::from(code)] * self.delta) >> 8).clamp(MS_MIN_DELTA, MS_MAX_DELTA);
        sample
    }

    fn expand(&mut self, code: u8) -> i16 {
        // Codes are two's complement nibbles.
        let signed_code = if code & 8 != 0 {
            i32::from(code) - 16
        } else {
            i32::from(code)
        };
        self.update(code, signed_code)
    }

    fn compress(&mut self, sample: i16) -> (u8, i32) {
        let error = i32::from(sample) - self.prediction();
        let bias = self.delta / 2;
        let rounded = if error < 0 { error - bias } else { error + bias };
        let signed_code = (rounded / self.delta).clamp(-8, 7);
        let code = u8::try_from(signed_code & 0x0F).expect("masked to a nibble");
        let reconstructed = self.update(code, signed_code);
        let distortion = i32::from(sample) - i32::from(reconstructed);
        (code, distortion)
    }
}

/// Microsoft ADPCM block decoder and encoder state.
#[derive(Debug, Clone)]
pub(super) struct MsAdpcm {
    channels: usize,
    block_align: usize,
    frames_per_block: usize,
    coefficients: Vec<(i16, i16)>,
}

impl MsAdpcm {
    pub(super) fn new(
        channels: usize,
        block_align: usize,
        frames_per_block: Option<usize>,
        coefficients: Vec<(i16, i16)>,
    ) -> Result<Self, CodecError> {
        if !(1..=2).contains(&channels) {
            return Err(CodecError::InvalidFormat(
                "Microsoft ADPCM supports one or two channels",
            ));
        }
        if block_align <= MS_HEADER_SIZE * channels {
            return Err(CodecError::InvalidFormat(
                "Microsoft ADPCM block alignment is smaller than its header",
            ));
        }
        if coefficients.is_empty() {
            return Err(CodecError::InvalidFormat(
                "Microsoft ADPCM format has no predictor coefficients",
            ));
        }
        let capacity = ms_frames_per_block(channels, block_align);
        let frames_per_block = frames_per_block.unwrap_or(capacity);
        if !(2 <= frames_per_block && frames_per_block <= capacity) {
            return Err(CodecError::InvalidFormat(
                "Microsoft ADPCM samples per block do not fit the block",
            ));
        }
        Ok(Self {
            channels,
            block_align,
            frames_per_block,
            coefficients,
        })
    }

    pub(super) fn block_align(&self) -> usize {
        self.block_align
    }

    pub(super) fn frames_per_block(&self) -> usize {
        self.frames_per_block
    }

    /// Decodes one block, which may be a truncated final block.
    pub(super) fn decode_block(&mut self, block: &[u8], out: &mut Vec<i16>) -> Result<(), CodecError> {
        let channels = self.channels;
        let Some((header, codes)) = block.split_at_checked(MS_HEADER_SIZE * channels) else {
            return Err(CodecError::InvalidData(
                "Microsoft ADPCM block is shorter than its header",
            ));
        };

        // Header fields are grouped by field, each holding one value per channel.
        let read_i16 = |field_offset: usize, channel: usize| {
            let offset = field_offset + 2 * channel;
            i32::from(i16::from_le_bytes([header[offset], header[offset + 1]]))
        };
        let mut state = Vec::with_capacity(channels);
        for (channel, predictor) in header[..channels].iter().enumerate() {
            let Some(&(coefficient1, coefficient2)) = self.coefficients.get(usize::from(*predictor)) else {
                return Err(CodecError::InvalidData(
                    "Microsoft ADPCM block selects an unknown predictor",
                ));
            };
            state.push(MsChannel {
                coefficient1: i32::from(coefficient1),
                coefficient2: i32::from(coefficient2),
                delta: read_i16(channels, channel),
                sample1: read_i16(3 * channels, channel),
                sample2: read_i16(5 * channels, channel),
            });
        }

        // The older sample is played first.
        out.extend(state.iter().map(|channel| clamp_i16(channel.sample2)));
        out.extend(state.iter().map(|channel| clamp_i16(channel.sample1)));

        // Codes alternate between channels, high nibble first.
        let code_limit = (self.frames_per_block - 2) * channels;
        let nibbles = codes.iter().flat_map(|byte| [byte >> 4, byte & 0x0F]).take(code_limit);
        for (index, code) in nibbles.enumerate() {
            out.push(state[index % channels].expand(code));
        }
        // A truncated block can end on a half frame; drop it so channels stay aligned.
        let partial = (out.len()) % channels;
        out.truncate(out.len() - partial);

        Ok(())
    }

    /// Encodes exactly one block of `frames_per_block()` interleaved frames.
    pub(super) fn encode_block(&mut self, frames: &[i16], out: &mut Vec<u8>) {
        let channels = self.channels;
        debug_assert_eq!(frames.len(), self.frames_per_block * channels);
        let block_start = out.len();

        // Pick, per channel, the predictor that reconstructs this block with the least distortion.
        let mut chosen = Vec::with_capacity(channels);
        for channel in 0..channels {
            let samples = || frames.iter().skip(channel).step_by(channels).copied();
            let (predictor, _) = self
                .coefficients
                .iter()
                .enumerate()
                .take(256)
                .map(|(predictor, coefficients)| {
                    let mut state = initial_ms_state(*coefficients, samples());
                    let distortion: i64 = samples()
                        .skip(2)
                        .map(|sample| {
                            let (_, error) = state.compress(sample);
                            i64::from(error) * i64::from(error)
                        })
                        .sum();
                    (predictor, distortion)
                })
                .min_by_key(|(_, distortion)| *distortion)
                .expect("coefficient list is not empty");
            chosen.push(predictor);
        }

        let mut state: Vec<MsChannel> = chosen
            .iter()
            .enumerate()
            .map(|(channel, predictor)| {
                initial_ms_state(
                    self.coefficients[*predictor],
                    frames.iter().skip(channel).step_by(channels).copied(),
                )
            })
            .collect();

        for predictor in &chosen {
            out.push(u8::try_from(*predictor).expect("at most 256 predictors are tried"));
        }
        let fields: [fn(&MsChannel) -> i32; 3] = [
            |channel| channel.delta,
            |channel| channel.sample1,
            |channel| channel.sample2,
        ];
        for field in fields {
            for channel in &state {
                out.extend_from_slice(&clamp_i16(field(channel)).to_le_bytes());
            }
        }

        let mut high = None;
        for (index, sample) in frames[2 * channels..].iter().enumerate() {
            let (code, _) = state[index % channels].compress(*sample);
            match high.take() {
                None => high = Some(code),
                Some(high) => out.push((high << 4) | code),
            }
        }
        if let Some(high) = high {
            out.push(high << 4);
        }
        // A block carrying fewer samples than it can hold is still `nBlockAlign` bytes long.
        out.resize(block_start + self.block_align, 0);
    }
}

/// Seeds an encoder channel from the first two samples of a block.
fn initial_ms_state(coefficients: (i16, i16), mut samples: impl Iterator<Item = i16>) -> MsChannel {
    let first = samples.next().unwrap_or(0);
    let second = samples.next().unwrap_or(first);
    let third = samples.next().unwrap_or(second);
    // Start the step size near the first prediction error so the block does not open with a ramp.
    let delta = ((i32::from(third) - i32::from(second)).abs() / 4).clamp(MS_MIN_DELTA, i32::from(i16::MAX));
    MsChannel {
        coefficient1: i32::from(coefficients.0),
        coefficient2: i32::from(coefficients.1),
        delta,
        sample1: i32::from(second),
        sample2: i32::from(first),
    }
}

fn clamp_i16(value: i32) -> i16 {
    i16::try_from(value.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).expect("clamped to i16")
}

fn clamp_i16_wide(value: i64) -> i16 {
    i16::try_from(value.clamp(i64::from(i16::MIN), i64::from(i16::MAX))).expect("clamped to i16")
}
