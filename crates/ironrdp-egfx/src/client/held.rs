//! Commands held back behind H.264 pictures that an asynchronous decoder has not returned yet.

use core::time::Duration;
use std::collections::{BTreeSet, VecDeque};

use ironrdp_core::{Encode as _, MonotonicInstant};
use ironrdp_pdu::geometry::ExclusiveRectangle;

use crate::decode::{DecodedPicture, DecoderError, DecoderResult};
use crate::pdu::GfxPdu;

/// Upper bound on the commands held back behind H.264 pictures still decoding.
const MAX_HELD_COMMANDS: usize = 16384;

/// Upper bound on the encoded size of the commands held back.
const MAX_HELD_COMMAND_BYTES: usize = 32 << 20;

/// Upper bound on the pictures submitted to a decoder and not returned yet, across all surfaces.
/// The server's frame acknowledgement window keeps a compliant server a few frames ahead.
const MAX_PENDING_PICTURES: usize = 16;

/// Upper bound on the bytes of the decoded pictures waiting behind an earlier one.
/// Sixteen 1080p RGBA pictures, or about forty in YUV 4:2:0.
const MAX_HELD_PICTURE_BYTES: usize = 128 << 20;

/// How long the graphics pipeline waits for an asynchronous H.264 decoder ([`H264Decoder::submit`]).
///
/// The wait of a picture starts when a [`GraphicsPipelineClient::poll_decoded`] finds it pending at
/// the head of the queue, not when it was submitted.
/// Pictures behind other stalled ones can therefore wait several deadlines before they are given up:
/// these bound each picture's wait at the head, not the time from submission to acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeTimeouts {
    /// After this wait, the decoder is asked to output what it holds back ([`H264Decoder::flush`]).
    pub flush_after: Duration,
    /// After this wait, the picture is given up: its update is dropped, its frame acknowledged, and its decoder reset.
    pub give_up_after: Duration,
}

impl DecodeTimeouts {
    /// Flushes after `flush_after` and gives up after `give_up_after`.
    pub fn new(flush_after: Duration, give_up_after: Duration) -> Self {
        Self {
            flush_after,
            give_up_after,
        }
    }
}

impl Default for DecodeTimeouts {
    /// Flushes after 100 ms, about three frames at 30 frames per second, and gives up after 2 s.
    fn default() -> Self {
        Self::new(Duration::from_millis(100), Duration::from_secs(2))
    }
}

/// The decoder a picture was submitted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum DecoderKey {
    /// The decoder passed to [`GraphicsPipelineClient::new`].
    Shared,
    /// The decoder the factory made for this surface.
    Surface(u16),
}

/// A command held back behind an H.264 picture that is still decoding.
pub(super) enum Held {
    /// An AVC420 picture already submitted to its decoder.
    Avc420(HeldPicture),
    /// A command to apply once everything before it applied.
    Pdu(GfxPdu),
}

pub(super) struct HeldPicture {
    pub(super) surface_id: u16,
    pub(super) decoder: DecoderKey,
    pub(super) destination_rectangle: ExclusiveRectangle,
    pub(super) region_rectangles: Vec<ExclusiveRectangle>,
    /// `None` until the decoder returns the picture.
    pub(super) frame: Option<DecoderResult<DecodedPicture>>,
    pub(super) waiting_since: Option<MonotonicInstant>,
    pub(super) flushed: bool,
}

impl HeldPicture {
    pub(super) fn new(
        surface_id: u16,
        decoder: DecoderKey,
        destination_rectangle: ExclusiveRectangle,
        region_rectangles: Vec<ExclusiveRectangle>,
        frame: Option<DecoderResult<DecodedPicture>>,
    ) -> Self {
        Self {
            surface_id,
            decoder,
            destination_rectangle,
            region_rectangles,
            frame,
            waiting_since: None,
            flushed: false,
        }
    }
}

/// The held commands, with the resources they retain.
#[derive(Default)]
pub(super) struct HeldQueue {
    /// INVARIANT: empty, or starts with the oldest picture not painted yet.
    pub(super) commands: VecDeque<Held>,
    /// Held commands that end an H.264 stream, so AVC420 pictures arriving after one are held
    /// undecoded until it applies: their decoder resets first, and must not have pictures pending then.
    stream_ends: usize,
    /// INVARIANT: the encoded size of the held `Held::Pdu` commands.
    command_bytes: usize,
    /// INVARIANT: the bytes of the held pictures already returned.
    picture_bytes: usize,
    /// INVARIANT: the held pictures not returned yet.
    pending: usize,
}

impl HeldQueue {
    pub(super) fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub(super) fn has_pending(&self) -> bool {
        0 < self.pending
    }

    pub(super) fn ends_stream_ahead(&self) -> bool {
        0 < self.stream_ends
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    fn charge(&mut self, picture: &HeldPicture) {
        match &picture.frame {
            None => self.pending += 1,
            Some(Ok(frame)) => self.picture_bytes += frame.allocated_bytes(),
            Some(Err(_)) => {}
        }
    }

    fn release(&mut self, picture: &HeldPicture) {
        match &picture.frame {
            None => self.pending -= 1,
            Some(Ok(frame)) => self.picture_bytes -= frame.allocated_bytes(),
            Some(Err(_)) => {}
        }
    }

    /// Holds a picture behind the held commands, or ahead of them when `front`.
    pub(super) fn push_picture(&mut self, picture: HeldPicture, front: bool) {
        self.charge(&picture);
        if front {
            self.commands.push_front(Held::Avc420(picture));
        } else {
            self.commands.push_back(Held::Avc420(picture));
        }
    }

    pub(super) fn push_command(&mut self, pdu: GfxPdu) {
        if ends_stream(&pdu) {
            self.stream_ends += 1;
        }
        self.command_bytes += pdu.size();
        self.commands.push_back(Held::Pdu(pdu));
    }

    pub(super) fn pop_front(&mut self) -> Option<Held> {
        let command = self.commands.pop_front()?;
        match &command {
            Held::Avc420(picture) => self.release(picture),
            Held::Pdu(pdu) => {
                if ends_stream(pdu) {
                    self.stream_ends -= 1;
                }
                self.command_bytes -= pdu.size();
            }
        }
        Some(command)
    }

    /// Takes what each decoder returned so far for its pending pictures, oldest first, so returned
    /// pictures count against the budget even while an earlier picture of another decoder is pending.
    pub(super) fn collect(&mut self, mut poll: impl FnMut(DecoderKey) -> Option<DecoderResult<DecodedPicture>>) {
        let mut exhausted = BTreeSet::new();
        for command in &mut self.commands {
            let Held::Avc420(picture) = command else {
                continue;
            };
            if picture.frame.is_some() || exhausted.contains(&picture.decoder) {
                continue;
            }
            match poll(picture.decoder) {
                Some(frame) => {
                    self.pending -= 1;
                    if let Ok(frame) = &frame {
                        self.picture_bytes += frame.allocated_bytes();
                    }
                    picture.frame = Some(frame);
                }
                None => {
                    exhausted.insert(picture.decoder);
                }
            }
        }
    }

    /// Sets the result of the oldest held picture, when it is pending.
    pub(super) fn resolve_front(&mut self, frame: DecoderResult<DecodedPicture>) {
        if let Some(Held::Avc420(picture)) = self.commands.front_mut()
            && picture.frame.is_none()
        {
            self.pending -= 1;
            if let Ok(frame) = &frame {
                self.picture_bytes += frame.allocated_bytes();
            }
            picture.frame = Some(frame);
        }
    }

    /// Sets the result of the pending held pictures submitted to a decoder in `decoders`.
    pub(super) fn resolve_pending(&mut self, decoders: &BTreeSet<DecoderKey>, reason: &str) {
        for command in &mut self.commands {
            if let Held::Avc420(picture) = command
                && picture.frame.is_none()
                && decoders.contains(&picture.decoder)
            {
                picture.frame = Some(Err(DecoderError::msg(reason.to_owned())));
                self.pending -= 1;
            }
        }
    }

    /// Whether the held commands retain more than their budget.
    pub(super) fn over_budget(&self) -> bool {
        MAX_HELD_COMMANDS < self.commands.len()
            || MAX_HELD_COMMAND_BYTES < self.command_bytes
            || MAX_HELD_PICTURE_BYTES < self.picture_bytes
            || MAX_PENDING_PICTURES < self.pending
    }
}

fn ends_stream(pdu: &GfxPdu) -> bool {
    matches!(
        pdu,
        GfxPdu::ResetGraphics(_) | GfxPdu::CreateSurface(_) | GfxPdu::DeleteSurface(_) | GfxPdu::CapabilitiesConfirm(_)
    )
}
