//! V2 acknowledgment payloads.
//!
//! `Acknowledgement Payload` (MS-RDPEUDP2 Section 2.2.1.2.1) and
//! `Acknowledgement Vector Payload` (MS-RDPEUDP2 Section 2.2.1.2.6).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

/// Maximum number of entries an [`AckVectorPayload`] can carry.
///
/// `codedAckVecSize` (MS-RDPEUDP2 Section 2.2.1.2.6) is a 7-bit field.
pub const ACK_VECTOR_MAX_ENTRIES: usize = 127;

// ── Acknowledgement Payload (ACK flag) ──

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Acknowledges one or more consecutively received packets.
///
/// MS-RDPEUDP2 Section 2.2.1.2.1.
/// Present when the ACK flag (0x001) is set in the header.
///
/// Wire layout:
/// - `SeqNum(2)` + `receivedTS(3)` + `sendAckTimeGap(1)` +
///   `numDelayedAcks(4 bits) : delayAckTimeScale(4 bits)` = 7 bytes fixed.
/// - `delayAckTimeAdditions(numDelayedAcks bytes)` = variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckPayload {
    /// Lower 16 bits of the acknowledged data packet's sequence number.
    pub seq_num: u16,

    /// Lower 24 bits of the timestamp when the packet was received.
    /// Units of 4 microseconds.
    pub received_ts: u32,

    /// Time in milliseconds between packet receipt and ACK send.
    pub send_ack_time_gap: u8,

    /// Scale for delay time additions (0..=15).
    /// Each addition is in units of `(1 << delay_ack_time_scale)` microseconds.
    pub delay_ack_time_scale: u8,

    /// Per-ACK time deltas in reverse chronological order.
    ///
    /// The wire format carries the count in the low nibble of the packed
    /// byte, so it is derived from this vector on encode rather than stored.
    /// At most 15 entries fit; `encode` rejects more.
    /// Each byte times `(1 << delay_ack_time_scale)` microseconds gives the
    /// time between adjacent acknowledgments.
    pub delay_ack_time_additions: Vec<u8>,
}

impl AckPayload {
    /// Fixed part size: SeqNum(2) + receivedTS(3) + sendAckTimeGap(1) + packed_nibbles(1) = 7.
    const FIXED_PART_SIZE: usize = 7;
    const NAME: &'static str = "RDP-UDP2 Acknowledgement Payload";

    /// Mask for the lower 24 bits of a timestamp.
    pub const TIMESTAMP_MASK: u32 = 0x00FF_FFFF;
}

impl Encode for AckPayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.seq_num);

        // receivedTS: 3 bytes little-endian (lower 24 bits)
        let ts_bytes = (self.received_ts & Self::TIMESTAMP_MASK).to_le_bytes();
        dst.write_u8(ts_bytes[0]);
        dst.write_u8(ts_bytes[1]);
        dst.write_u8(ts_bytes[2]);

        dst.write_u8(self.send_ack_time_gap);

        // Pack numDelayedAcks(high nibble) and delayAckTimeScale(low nibble).
        // The count comes from the vector, never from a separate field: the
        // two can disagree, and a truncated count leaves the decoder reading
        // entries that were never written.
        let num_delayed_acks = u8::try_from(self.delay_ack_time_additions.len())
            .ok()
            .filter(|count| *count <= 0x0F)
            .ok_or_else(|| {
                ironrdp_core::invalid_field_err!("AckPayload", "numDelayedAcks", "exceeds 4-bit maximum (15)")
            })?;
        // [MS-RDPEUDP2] 2.2.1.2.1: numDelayedAcks occupies bits 48 to 51 and
        // delayAckTimeScale bits 52 to 55. Bits are numbered
        // least-significant first, so the count is the low nibble.
        let packed = ((self.delay_ack_time_scale & 0x0F) << 4) | num_delayed_acks;
        dst.write_u8(packed);

        dst.write_slice(&self.delay_ack_time_additions);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.delay_ack_time_additions.len()
    }
}

impl Decode<'_> for AckPayload {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);

        let seq_num = src.read_u16();

        // receivedTS: 3 bytes little-endian
        let ts_b0 = u32::from(src.read_u8());
        let ts_b1 = u32::from(src.read_u8());
        let ts_b2 = u32::from(src.read_u8());
        let received_ts = ts_b0 | (ts_b1 << 8) | (ts_b2 << 16);

        let send_ack_time_gap = src.read_u8();

        let packed = src.read_u8();
        let num_delayed_acks = packed & 0x0F;
        let delay_ack_time_scale = (packed >> 4) & 0x0F;

        let additions_len = usize::from(num_delayed_acks);
        ironrdp_core::ensure_size!(in: src, size: additions_len);
        let delay_ack_time_additions = src.read_slice(additions_len).to_vec();

        Ok(Self {
            seq_num,
            received_ts,
            send_ack_time_gap,
            delay_ack_time_scale,
            delay_ack_time_additions,
        })
    }
}

// ── Acknowledgement Vector Payload (ACKVEC flag) ──

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Per-packet acknowledgment state encoding mode.
///
/// MS-RDPEUDP2 Section 2.2.1.2.6.
/// Each byte in the coded ACK vector is independently encoded as either
/// a state-map (MSB = 0) or a run-length (MSB = 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckVectorEntry {
    /// State-map mode (MSB = 0).
    /// Remaining 7 bits are a bitmap: each bit represents one sequence number
    /// (1 = received, 0 = not received), from lowest to highest bit.
    StateMap {
        /// 7-bit bitmap of received/not-received states.
        bitmap: u8,
    },

    /// Run-length mode (MSB = 1).
    /// Bit 6 = state (1 = received, 0 = not received).
    /// Bits 0-5 = run length (number of consecutive packets in this state).
    RunLength {
        /// Whether the packets in this run were received.
        received: bool,
        /// Number of consecutive packets in this state (0..=63).
        length: u8,
    },
}

impl AckVectorEntry {
    /// Encode to a single byte.
    pub fn to_byte(self) -> u8 {
        match self {
            Self::StateMap { bitmap } => bitmap & 0x7F, // MSB = 0
            Self::RunLength { received, length } => {
                let mut byte = 0x80; // MSB = 1
                if received {
                    byte |= 0x40; // bit 6 = state
                }
                byte | (length & 0x3F)
            }
        }
    }

    /// Decode from a single byte.
    pub fn from_byte(byte: u8) -> Self {
        if (byte & 0x80) == 0 {
            // State-map mode
            Self::StateMap { bitmap: byte & 0x7F }
        } else {
            // Run-length mode
            Self::RunLength {
                received: (byte & 0x40) != 0,
                length: byte & 0x3F,
            }
        }
    }

    /// Number of sequence numbers covered by this entry.
    pub fn coverage(self) -> u16 {
        match self {
            Self::StateMap { .. } => 7,
            Self::RunLength { length, .. } => u16::from(length),
        }
    }
}

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Vector of per-packet acknowledgment states within the receiver window.
///
/// MS-RDPEUDP2 Section 2.2.1.2.6.
/// Present when the ACKVEC flag (0x008) is set in the header.
/// Mutually exclusive with the ACK payload.
///
/// Wire layout:
/// - `BaseSeqNum(2)` + packed byte (`codedAckVecSize(7 bits) : TimeStampPresent(1 bit)`) = 3 bytes.
/// - If `TimeStampPresent`: `TimeStamp(3)` + `SendAckTimeGapInMs(1)` = 4 bytes.
/// - `codedAckVector(codedAckVecSize bytes)`.
///
/// `codedAckVecSize` is 7 bits, so at most [`ACK_VECTOR_MAX_ENTRIES`] entries fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckVectorPayload {
    /// Lower 16 bits of the base sequence number for this vector.
    pub base_seq_num: u16,

    /// Lower 24 bits of timestamp for the highest unacked sequence number received.
    /// Units of 4 microseconds. Present only when `timestamp_present`.
    pub timestamp: Option<u32>,

    /// Time in ms between ACK send and last data packet receipt.
    /// 255 = invalid/unused. Present only when `timestamp_present`.
    pub send_ack_time_gap_ms: Option<u8>,

    /// Encoded ACK vector entries.
    pub entries: Vec<AckVectorEntry>,
}

impl AckVectorPayload {
    const NAME: &'static str = "RDP-UDP2 Acknowledgement Vector Payload";

    /// Minimum fixed part: BaseSeqNum(2) + packed_byte(1) = 3 bytes.
    const MIN_FIXED_SIZE: usize = 3;

    /// Optional timestamp block: TimeStamp(3) + SendAckTimeGapInMs(1) = 4 bytes.
    const TIMESTAMP_BLOCK_SIZE: usize = 4;
}

impl Encode for AckVectorPayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_size!(in: dst, size: self.size());

        for entry in &self.entries {
            match *entry {
                AckVectorEntry::StateMap { bitmap } if bitmap > 0x7F => {
                    return Err(ironrdp_core::invalid_field_err!(
                        "AckVectorPayload",
                        "entries",
                        "StateMap bitmap exceeds 7-bit maximum (0x7F)"
                    ));
                }
                AckVectorEntry::RunLength { length, .. } if length > 0x3F => {
                    return Err(ironrdp_core::invalid_field_err!(
                        "AckVectorPayload",
                        "entries",
                        "RunLength length exceeds 6-bit maximum (0x3F)"
                    ));
                }
                _ => {}
            }
        }

        // The timestamp block is one unit on the wire (see below): a value
        // carrying only one half has nowhere valid to go. Silently dropping
        // it would discard data the caller explicitly set.
        if self.timestamp.is_some() != self.send_ack_time_gap_ms.is_some() {
            return Err(ironrdp_core::invalid_field_err!(
                "AckVectorPayload",
                "timestamp",
                "timestamp and send_ack_time_gap_ms must be both present or both absent"
            ));
        }

        dst.write_u16(self.base_seq_num);

        let vec_size: u8 = ironrdp_core::cast_length!("AckVectorPayload", "codedAckVecSize", self.entries.len())?;
        if usize::from(vec_size) > ACK_VECTOR_MAX_ENTRIES {
            return Err(ironrdp_core::invalid_field_err!(
                "AckVectorPayload",
                "codedAckVecSize",
                "exceeds 7-bit maximum (127)"
            ));
        }

        // The timestamp block is one unit on the wire: the gap byte and the
        // three timestamp bytes are written together and read together. It is
        // therefore present only when both fields are, and the flag bit, the
        // write below and `size` must all agree on that. They did not: the flag
        // was set from `timestamp` alone, so a value carrying a timestamp
        // without a gap advertised a block it never wrote, and a peer would
        // have read the first four ACK entries as the timestamp.
        let timestamp_present = self.timestamp.is_some() && self.send_ack_time_gap_ms.is_some();
        let packed = (vec_size & 0x7F) | if timestamp_present { 0x80 } else { 0x00 };
        dst.write_u8(packed);

        if let (Some(ts), Some(gap)) = (self.timestamp, self.send_ack_time_gap_ms) {
            // [MS-RDPEUDP2] 2.2.1.2.6 places TimeStamp (bits 24 to 47) before
            // SendAckTimeGapInMs (bits 48 to 55).
            let ts_bytes = (ts & AckPayload::TIMESTAMP_MASK).to_le_bytes();
            dst.write_u8(ts_bytes[0]);
            dst.write_u8(ts_bytes[1]);
            dst.write_u8(ts_bytes[2]);
            dst.write_u8(gap);
        }

        for entry in &self.entries {
            dst.write_u8(entry.to_byte());
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let mut total = Self::MIN_FIXED_SIZE;
        // Same condition as `encode`: the block goes out only when both halves
        // are present.
        if self.timestamp.is_some() && self.send_ack_time_gap_ms.is_some() {
            total += Self::TIMESTAMP_BLOCK_SIZE;
        }
        total += self.entries.len();
        total
    }
}

impl Decode<'_> for AckVectorPayload {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_size!(in: src, size: Self::MIN_FIXED_SIZE);

        let base_seq_num = src.read_u16();

        let packed = src.read_u8();
        let coded_ack_vec_size = packed & 0x7F;
        let timestamp_present = (packed & 0x80) != 0;

        let (timestamp, send_ack_time_gap_ms) = if timestamp_present {
            ironrdp_core::ensure_size!(in: src, size: Self::TIMESTAMP_BLOCK_SIZE);
            let ts_b0 = u32::from(src.read_u8());
            let ts_b1 = u32::from(src.read_u8());
            let ts_b2 = u32::from(src.read_u8());
            let ts = ts_b0 | (ts_b1 << 8) | (ts_b2 << 16);
            let gap = src.read_u8();
            (Some(ts), Some(gap))
        } else {
            (None, None)
        };

        let entries_len = usize::from(coded_ack_vec_size);
        ironrdp_core::ensure_size!(in: src, size: entries_len);
        let mut entries = Vec::with_capacity(entries_len);
        for _ in 0..entries_len {
            entries.push(AckVectorEntry::from_byte(src.read_u8()));
        }

        Ok(Self {
            base_seq_num,
            timestamp,
            send_ack_time_gap_ms,
            entries,
        })
    }
}
