//! Playout policy for the jitter buffer between received wave blocks and an audio device.
//!
//! Wave blocks arrive in bursts and on the server's clock, while the device consumes frames at
//! a steady rate on its own clock. [`PlayoutController`] decides, for every device period, how many
//! buffered frames to play so that the buffer settles at a latency target:
//!
//! - it builds up the target before playing, and again after an underrun, but starts anyway once
//!   the buffer has stopped growing for as long as the target, so a short sound is not held back;
//! - it absorbs clock drift by playing 0.5% more or fewer frames than the period needs while the
//!   smoothed buffer level stays outside a tolerance band around the target;
//! - it discards a backlog far above the target at once, so a network stall does not leave audio
//!   lagging behind video for the rest of the session. Audio already buffered when playback starts
//!   (for example while the device was opening) is played rather than discarded, and drift
//!   correction drains it.
//!
//! The controller only does arithmetic on frame counts; [`stretch`] resamples the frames it selects
//! to the period length.

use core::time::Duration;

/// Smoothing factor of the buffer level, as a power of two (the level moves 1/16 of the way to each
/// new sample).
const LEVEL_SMOOTHING_SHIFT: u32 = 4;

/// Fixed-point fraction bits of the smoothed level.
const LEVEL_FRACTION_BITS: u32 = 8;

/// Fraction of a period added or removed while correcting drift (1/200, so 0.5%).
const DRIFT_CORRECTION_DIVISOR: usize = 200;

/// What the device callback does with one period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// Output silence without consuming frames while the buffer fills.
    Silence,
    /// Discard `skip` frames, then consume `consume` frames and stretch them to the period.
    Play { skip: usize, consume: usize },
    /// Play the `consume` frames left, followed by silence, and refill the buffer.
    Underrun { consume: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Buffering,
    Playing,
}

/// Jitter buffer policy; see the [module documentation](self).
#[derive(Debug, Clone)]
pub struct PlayoutController {
    /// Buffer level to settle at, in frames.
    target: usize,
    /// Distance from the target the smoothed level may drift before it is corrected, in frames.
    tolerance: usize,
    /// Level above which a backlog is discarded at once, in frames.
    resync_level: usize,
    /// Current discard threshold: `resync_level`, or higher while a backlog present at the start of
    /// playback drains.
    ceiling: usize,
    state: State,
    /// Smoothed buffer level, in frames scaled by `2^LEVEL_FRACTION_BITS`.
    level: u64,
    /// While buffering: the level seen in the previous period, and how long it has not changed, in
    /// frames.
    buffering_level: usize,
    buffering_stalled: usize,
}

/// Number of frames `duration` spans at `sample_rate` frames per second, saturating.
pub fn frames_in(duration: Duration, sample_rate: u32) -> usize {
    let frames = duration.as_micros() * u128::from(sample_rate) / 1_000_000;
    usize::try_from(frames).unwrap_or(usize::MAX)
}

impl PlayoutController {
    /// Creates a controller for a device running at `sample_rate` frames per second.
    ///
    /// `target` is the buffered audio to keep ahead of the device; it trades latency for
    /// resilience to network jitter.
    pub fn new(sample_rate: u32, target: Duration) -> Self {
        let frames = |duration: Duration| frames_in(duration, sample_rate);
        let target = frames(target).max(1);
        let tolerance = (target / 4).max(frames(Duration::from_millis(10)));
        let backlog = target.max(frames(Duration::from_millis(200)));
        Self {
            target,
            tolerance,
            resync_level: target + tolerance + backlog,
            ceiling: target + tolerance + backlog,
            state: State::Buffering,
            level: 0,
            buffering_level: 0,
            buffering_stalled: 0,
        }
    }

    /// Target buffer level, in frames.
    pub fn target(&self) -> usize {
        self.target
    }

    /// Returns whether the controller is building up the buffer rather than playing.
    pub fn is_buffering(&self) -> bool {
        self.state == State::Buffering
    }

    /// Decides what to do with a device period of `requested` frames when `buffered` frames are
    /// available.
    ///
    /// The returned frame counts never exceed `buffered`.
    pub fn next_period(&mut self, buffered: usize, requested: usize) -> Period {
        if self.state == State::Buffering {
            if buffered < self.target.max(requested) {
                // A sound shorter than the target never fills the buffer: play what arrived once
                // nothing more has come for as long as the target.
                if buffered != 0 && buffered == self.buffering_level {
                    self.buffering_stalled = self.buffering_stalled.saturating_add(requested);
                } else {
                    self.buffering_stalled = 0;
                }
                self.buffering_level = buffered;
                if buffered == 0 || self.buffering_stalled < self.target {
                    return Period::Silence;
                }
            }
            self.buffering_level = 0;
            self.buffering_stalled = 0;
            self.state = State::Playing;
            self.level = scaled(buffered);
            self.ceiling = self.resync_level.max(buffered + self.tolerance);
        }

        if buffered < requested {
            self.state = State::Buffering;
            return Period::Underrun { consume: buffered };
        }

        // Exponential moving average; the level is non-negative, so the arithmetic stays unsigned.
        let sample = scaled(buffered);
        if self.level < sample {
            self.level += (sample - self.level) >> LEVEL_SMOOTHING_SHIFT;
        } else {
            self.level -= (self.level - sample) >> LEVEL_SMOOTHING_SHIFT;
        }

        // The ceiling follows a draining startup backlog down to the configured level.
        let level_frames = usize::try_from(self.level >> LEVEL_FRACTION_BITS).unwrap_or(usize::MAX);
        self.ceiling = self
            .resync_level
            .max(self.ceiling.min(level_frames.saturating_add(self.tolerance)));

        if self.ceiling < buffered {
            let skip = buffered - requested - self.target.min(buffered - requested);
            self.level = scaled(self.target + requested);
            self.ceiling = self.resync_level;
            return Period::Play {
                skip,
                consume: requested,
            };
        }

        let correction = (requested / DRIFT_CORRECTION_DIVISOR).max(1);
        let consume = if scaled(self.target + self.tolerance) < self.level {
            (requested + correction).min(buffered)
        } else if self.level < scaled(self.target.saturating_sub(self.tolerance)) && correction < requested {
            requested - correction
        } else {
            requested
        };
        Period::Play { skip: 0, consume }
    }
}

fn scaled(frames: usize) -> u64 {
    u64::try_from(frames).unwrap_or(u64::MAX >> LEVEL_FRACTION_BITS) << LEVEL_FRACTION_BITS
}

/// Resamples interleaved `input` frames to fill `output` by linear interpolation.
///
/// The output runs from the first input frame to the last one (up to fixed-point rounding), so
/// consecutive calls on consecutive input stay continuous. Empty input fills `output` with silence.
pub fn stretch(input: &[i16], channels: usize, output: &mut [i16]) {
    let channels = channels.max(1);
    let in_frames = input.len() / channels;
    let out_frames = output.len() / channels;
    // Samples past the last whole output frame stay silent.
    let (frames, tail) = output.split_at_mut(out_frames * channels);
    tail.fill(0);

    if in_frames == 0 {
        frames.fill(0);
        return;
    }
    if in_frames == out_frames {
        frames.copy_from_slice(&input[..frames.len()]);
        return;
    }

    // Position of each output frame in the input, in 16.16 fixed point.
    let step = if out_frames < 2 {
        0
    } else {
        ((in_frames - 1) << 16) / (out_frames - 1)
    };
    let last = in_frames - 1;
    for (index, frame) in frames.chunks_exact_mut(channels).enumerate() {
        let position = index * step;
        let base = (position >> 16).min(last);
        let next = (base + 1).min(last);
        let weight = i64::from(u16::try_from(position & 0xFFFF).unwrap_or(u16::MAX));
        let from = &input[base * channels..][..channels];
        let to = &input[next * channels..][..channels];
        for ((sample, from), to) in frame.iter_mut().zip(from).zip(to) {
            // Interpolating between two i16 samples stays within their range.
            let value = i64::from(*from) + (((i64::from(*to) - i64::from(*from)) * weight) >> 16);
            *sample = i16::try_from(value).unwrap_or_default();
        }
    }
}
