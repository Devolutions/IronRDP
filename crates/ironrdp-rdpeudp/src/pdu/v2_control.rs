//! V2 control payloads: OverheadSize, DelayAckInfo, AckOfAcks.
//!
//! These are small fixed-size payloads used for flow control and
//! protocol tuning during data transfer.

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

// ── OverheadSize Payload ──

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Average extra header bytes from the RDP-UDP2 layer to the raw UDP layer.
///
/// MS-RDPEUDP2 Section 2.2.1.2.2.
/// Present when the OVERHEADSIZE flag (0x040) is set.
/// Sent by the Receiver to help the Sender's congestion control
/// accurately estimate available bandwidth.
///
/// Wire layout: `OverheadSize(1)` = 1 byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverheadSizePayload {
    /// Average header overhead in bytes.
    pub overhead_size: u8,
}

impl OverheadSizePayload {
    const FIXED_PART_SIZE: usize = 1;
    const NAME: &'static str = "RDP-UDP2 OverheadSize Payload";
}

impl Encode for OverheadSizePayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);
        dst.write_u8(self.overhead_size);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for OverheadSizePayload {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);
        let overhead_size = src.read_u8();
        Ok(Self { overhead_size })
    }
}

// ── DelayAckInfo Payload ──

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Parameters controlling the Receiver's ACK batching behavior.
///
/// MS-RDPEUDP2 Section 2.2.1.2.3.
/// Present when the DELAYACKINFO flag (0x100) is set.
/// Sent by the Sender to configure how the Receiver batches ACKs.
///
/// Wire layout (3 bytes):
/// - `MaxDelayedAcks(8 bits)` = 1 byte.
/// - `DelayedAckTimeoutInMs(16 bits)` = 2 bytes.
///
/// Default values (assumed after initialization):
/// - `MaxDelayedAcks = 8`
/// - `DelayedAckTimeoutInMs = RTT / 2`
///
/// The Sender can adjust these at any time during the connection.
/// If the Receiver delays beyond these parameters, the packet is
/// considered lost and must be retransmitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelayAckInfoPayload {
    /// Maximum number of ACKs the Receiver may batch before
    /// sending a standalone acknowledgment. Range 0..=255.
    pub max_delayed_acks: u8,

    /// Timeout in milliseconds. If the Receiver has unacknowledged
    /// packets older than this, it must send an ACK immediately.
    pub delayed_ack_timeout_ms: u16,
}

impl DelayAckInfoPayload {
    const FIXED_PART_SIZE: usize = 3;
    const NAME: &'static str = "RDP-UDP2 DelayAckInfo Payload";

    /// Default max delayed ACKs per MS-RDPEUDP2 Section 3.1.5.2.
    pub const DEFAULT_MAX_DELAYED_ACKS: u8 = 8;
}

impl Encode for DelayAckInfoPayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);

        // [MS-RDPEUDP2] 2.2.1.2.3 gives MaxDelayedAcks a whole byte, not a
        // nibble, so the full range 0..=255 goes out.
        dst.write_u8(self.max_delayed_acks);
        dst.write_u16(self.delayed_ack_timeout_ms);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for DelayAckInfoPayload {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);

        let max_delayed_acks = src.read_u8();
        let delayed_ack_timeout_ms = src.read_u16();

        Ok(Self {
            max_delayed_acks,
            delayed_ack_timeout_ms,
        })
    }
}

// ── AckOfAcks Payload ──

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Tells the Receiver the lowest sequence number the Sender still cares about.
///
/// MS-RDPEUDP2 Section 2.2.1.2.4.
/// Present when the AOA flag (0x010) is set.
/// Sent by the Sender to allow the Receiver to shrink its window buffer.
///
/// Wire layout: `AckOfAcksSeqNum(2)` = 2 bytes.
///
/// The Sender sends this after detecting packet loss to inform the Receiver
/// that packets with lower sequence numbers need not be tracked anymore.
/// The Sender should stop sending this once it receives an ACK or ACK vector
/// confirming a higher sequence number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AckOfAcksPayload {
    /// Lower 16 bits of the AckOfAcks sequence number.
    pub ack_of_acks_seq_num: u16,
}

impl AckOfAcksPayload {
    const FIXED_PART_SIZE: usize = 2;
    const NAME: &'static str = "RDP-UDP2 AckOfAcks Payload";
}

impl Encode for AckOfAcksPayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);
        dst.write_u16(self.ack_of_acks_seq_num);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for AckOfAcksPayload {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);
        let ack_of_acks_seq_num = src.read_u16();
        Ok(Self { ack_of_acks_seq_num })
    }
}
