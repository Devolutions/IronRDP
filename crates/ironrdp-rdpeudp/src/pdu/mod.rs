//! RDPEUDP PDU definitions.
//!
//! V1 handshake format (MS-RDPEUDP Section 2.2) and
//! V2 data transfer format (MS-RDPEUDP2 Section 2.2).
//!
//! The v1 format is used for the three-way handshake (SYN/SYN+ACK/ACK)
//! even when both sides negotiate v2+ for data transfer. Once the
//! handshake completes, all subsequent packets use the v2 wire format
//! from MS-RDPEUDP2.
//!
//! Types are organized by protocol version:
//! - `v1_*` modules: MS-RDPEUDP handshake structures.
//! - `v2_*` modules: MS-RDPEUDP2 data transfer structures.
//! - `prefix`: PacketPrefixByte wire-level framing for v2 packets.

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

// ── V1 Handshake modules ──

pub mod v1_ack;
pub mod v1_flags;
pub mod v1_header;
pub mod v1_syn;

// ── V2 Data Transfer modules ──

pub mod v2_ack;
pub mod v2_control;
pub mod v2_data;
pub mod v2_flags;
pub mod v2_header;

// ── Wire-level framing ──

pub mod prefix;

// ── V1 re-exports ──

// ── Prefix re-exports ──
pub use prefix::{PacketPrefixByte, PrefixError, decode_with_prefix, encode_with_prefix};
pub use v1_ack::{CorrelationIdPayload, V1AckOfAcksHeader, V1AckVectorElement, V1AckVectorHeader, VectorElementState};
pub use v1_flags::V1Flags;
pub use v1_header::FecHeader;
pub use v1_syn::{MTU_MAX, MTU_MIN, SynDataExPayload, SynDataPayload, SynExFlags, UdpVersion};
// ── V2 re-exports ──
pub use v2_ack::{AckPayload, AckVectorEntry, AckVectorPayload};
pub use v2_control::{AckOfAcksPayload, DelayAckInfoPayload, OverheadSizePayload};
pub use v2_data::{DataBody, DataHeader};
pub use v2_flags::V2Flags;
pub use v2_header::{LOG_WINDOW_SIZE_MAX, V2Header};

// ════════════════════════════════════════════════════════════════════
// Composite V1 Datagram
// ════════════════════════════════════════════════════════════════════

/// V1 flags that don't gate any optional payload; preserved on encode.
///
/// DATA and FEC are payload-gating but have no corresponding fields
/// in V1Datagram (v1 data transfer is not supported).
const V1_STANDALONE_FLAGS: u16 = V1Flags::FIN.bits()
    | V1Flags::CN.bits()
    | V1Flags::CWR.bits()
    | V1Flags::SACK_OPTION.bits()
    | V1Flags::SYNLOSSY.bits()
    | V1Flags::ACKDELAYED.bits();

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// A complete v1 datagram (SYN, SYN+ACK, or ACK).
///
/// MS-RDPEUDP Section 2.2.
/// The FecHeader's flags field determines which optional payloads
/// are present. On encode, payload-gating flags are automatically
/// derived from which `Option` fields are populated; standalone flags
/// (FIN, CN, CWR, SACK_OPTION, SYNLOSSY, ACKDELAYED) are preserved
/// from the header.
///
/// Wire payload ordering (per MS-RDPEUDP Section 2.2.2):
/// 1. FecHeader (mandatory, 8 bytes)
/// 2. V1AckVectorHeader (if ACK flag and not SYN, see below)
/// 3. V1AckOfAcksHeader (if ACK_OF_ACKS flag)
/// 4. SynDataPayload (if SYN flag)
/// 5. CorrelationIdPayload (if CORRELATION_ID flag)
/// 6. SynDataExPayload (if SYNEX flag)
///
/// A SYN+ACK is the exception to the ACK flag's usual meaning. Section
/// 2.2.2.1 defines the flag as "the ACK vector is present", but 3.1.5.1.3
/// builds the SYN+ACK as a plain SYN with the ACK flag set and snSourceAck
/// filled in, and nothing else. The capture in section 4.1.2 confirms it:
/// uFlags is 0x0005 (SYN | ACK) and the SYNDATA payload follows the header
/// directly, with no ACK vector between them. So on a SYN+ACK the flag says
/// only that snSourceAck is meaningful, and `ack_vector` must be `None`.
///
/// V1 data payloads (SOURCE_PAYLOAD / FEC_PAYLOAD) are not represented;
/// this crate always negotiates v2+ for data transfer.
///
/// `encode` does not zero-pad SYN and SYN+ACK datagrams to the negotiated
/// MTU that MS-RDPEUDP 3.1.5.1.1 and 3.1.5.1.3 require: this crate has no
/// socket to size the padding against. A caller sending these bytes on the
/// wire is responsible for padding to the smaller of `upstream_mtu` and
/// `downstream_mtu` before transmitting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1Datagram {
    /// Mandatory FecHeader. Standalone flags (FIN, CN, CWR, etc.)
    /// are preserved; payload-gating flags are recomputed on encode.
    pub header: FecHeader,

    /// ACK vector (run-length encoded receiver state).
    /// Gated by `V1Flags::ACK`, except on a SYN+ACK, which sets that flag
    /// without carrying a vector. Must be `None` whenever `syn_data` is set.
    pub ack_vector: Option<V1AckVectorHeader>,

    /// AckOfAcks (resets ACK vector encoding base).
    /// Gated by `V1Flags::ACK_OF_ACKS`.
    pub ack_of_acks: Option<V1AckOfAcksHeader>,

    /// SYN data (ISN, MTU).
    /// Gated by `V1Flags::SYN`.
    pub syn_data: Option<SynDataPayload>,

    /// Correlation ID for TCP/UDP binding.
    /// Gated by `V1Flags::CORRELATION_ID`.
    pub correlation_id: Option<CorrelationIdPayload>,

    /// Extended SYN data (version negotiation).
    /// Gated by `V1Flags::SYNEX`.
    pub syn_data_ex: Option<SynDataExPayload>,
}

impl V1Datagram {
    const NAME: &'static str = "V1 Datagram";

    /// Compute the flags from populated fields, preserving standalone flags.
    fn compute_flags(&self) -> V1Flags {
        let mut flags = V1Flags::from_bits_truncate(self.header.flags.bits() & V1_STANDALONE_FLAGS);

        if self.ack_vector.is_some() {
            flags |= V1Flags::ACK;
        }
        if self.ack_of_acks.is_some() {
            flags |= V1Flags::ACK_OF_ACKS;
        }
        if self.syn_data.is_some() {
            flags |= V1Flags::SYN;
            // On a SYN+ACK the ACK flag has no payload to be derived from,
            // so take it from the caller. Section 3.1.5.1.3 requires it to
            // be set on the server's half of the handshake.
            flags |= self.header.flags & V1Flags::ACK;
        }
        if self.correlation_id.is_some() {
            flags |= V1Flags::CORRELATION_ID;
        }
        if self.syn_data_ex.is_some() {
            flags |= V1Flags::SYNEX;
        }

        flags
    }
}

impl Encode for V1Datagram {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        // A SYN+ACK carries no ACK vector (3.1.5.1.3, capture in 4.1.2), and
        // a receiver keying off the SYN flag would read whatever we wrote here
        // as the start of the SYNDATA payload.
        if self.syn_data.is_some() && self.ack_vector.is_some() {
            return Err(ironrdp_core::invalid_field_err!(
                Self::NAME,
                "ack_vector",
                "a SYN datagram cannot carry an ACK vector"
            ));
        }

        ironrdp_core::ensure_size!(in: dst, size: self.size());

        // Write header with auto-computed flags
        let header = FecHeader {
            flags: self.compute_flags(),
            ..self.header
        };
        header.encode(dst)?;

        // Write payloads in spec-mandated order (MS-RDPEUDP Section 2.2.2)
        if let Some(ref ack_vector) = self.ack_vector {
            ack_vector.encode(dst)?;
        }
        if let Some(ref ack_of_acks) = self.ack_of_acks {
            ack_of_acks.encode(dst)?;
        }
        if let Some(ref syn_data) = self.syn_data {
            syn_data.encode(dst)?;
        }
        if let Some(ref correlation_id) = self.correlation_id {
            correlation_id.encode(dst)?;
        }
        if let Some(ref syn_data_ex) = self.syn_data_ex {
            syn_data_ex.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let mut total = self.header.size();
        if let Some(ref av) = self.ack_vector {
            total += av.size();
        }
        if let Some(ref aoa) = self.ack_of_acks {
            total += aoa.size();
        }
        if let Some(ref sd) = self.syn_data {
            total += sd.size();
        }
        if let Some(ref cid) = self.correlation_id {
            total += cid.size();
        }
        if let Some(ref sdex) = self.syn_data_ex {
            total += sdex.size();
        }
        total
    }
}

impl Decode<'_> for V1Datagram {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = FecHeader::decode(src)?;

        // V1 data payloads are not supported; reject if present since we
        // cannot skip them without knowing their wire size.
        if header.flags.contains(V1Flags::DATA) {
            return Err(ironrdp_core::invalid_field_err!(
                "V1 Datagram",
                "flags",
                "DATA flag is not supported in handshake datagrams"
            ));
        }
        if header.flags.contains(V1Flags::FEC) {
            return Err(ironrdp_core::invalid_field_err!(
                "V1 Datagram",
                "flags",
                "FEC flag is not supported in handshake datagrams"
            ));
        }

        // Decode payloads in spec-mandated order, gated by flags.
        //
        // SYN suppresses the ACK vector: on a SYN+ACK the ACK flag marks
        // snSourceAck as meaningful rather than announcing a vector. See the
        // note on `V1Datagram`.
        let ack_vector = if header.flags.contains(V1Flags::ACK) && !header.flags.contains(V1Flags::SYN) {
            Some(V1AckVectorHeader::decode(src)?)
        } else {
            None
        };

        let ack_of_acks = if header.flags.contains(V1Flags::ACK_OF_ACKS) {
            Some(V1AckOfAcksHeader::decode(src)?)
        } else {
            None
        };

        let syn_data = if header.flags.contains(V1Flags::SYN) {
            Some(SynDataPayload::decode(src)?)
        } else {
            None
        };

        let correlation_id = if header.flags.contains(V1Flags::CORRELATION_ID) {
            Some(CorrelationIdPayload::decode(src)?)
        } else {
            None
        };

        let syn_data_ex = if header.flags.contains(V1Flags::SYNEX) {
            // Same SYN-without-ACK direction test as the ack_vector gate above:
            // cookieHash rides only on a client-to-server SYN (MS-RDPEUDP
            // 2.2.2.9), never on a SYN+ACK.
            let is_client_syn = header.flags.contains(V1Flags::SYN) && !header.flags.contains(V1Flags::ACK);
            Some(SynDataExPayload::decode_directional(src, is_client_syn)?)
        } else {
            None
        };

        Ok(Self {
            header,
            ack_vector,
            ack_of_acks,
            syn_data,
            correlation_id,
            syn_data_ex,
        })
    }
}

// ════════════════════════════════════════════════════════════════════
// Composite V2 Packet
// ════════════════════════════════════════════════════════════════════

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// A complete RDP-UDP2 packet (after prefix byte extraction).
///
/// MS-RDPEUDP2 Section 2.2.1.
/// The V2Header's flags field determines which optional payloads
/// are present. On encode, payload-gating flags are auto-derived;
/// there are no standalone flags to preserve.
///
/// Wire payload ordering (per MS-RDPEUDP2 Section 2.2.1):
/// 1. V2Header (mandatory, 2 bytes)
/// 2. AckPayload (if ACK flag)
/// 3. OverheadSizePayload (if OVERHEADSIZE flag)
/// 4. DelayAckInfoPayload (if DELAYACKINFO flag)
/// 5. AckOfAcksPayload (if AOA flag)
/// 6. DataHeader (if DATA flag)
/// 7. AckVectorPayload (if ACKVEC flag)
/// 8. DataBody (if DATA flag), always last, extending to the end of the packet
///
/// Invariants:
/// - `ack` and `ack_vector` are mutually exclusive.
/// - `data_header` and `data_body` must be both present or both absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Packet {
    /// Mandatory header. Only `log_window_size` is taken from this field
    /// as given; the flags are always derived from which payloads are
    /// present and overwrite whatever `header.flags` was set to.
    pub header: V2Header,

    /// ACK payload. Gated by `V2Flags::ACK`.
    /// Mutually exclusive with `ack_vector`.
    pub ack: Option<AckPayload>,

    /// OverheadSize payload. Gated by `V2Flags::OVERHEADSIZE`.
    pub overhead_size: Option<OverheadSizePayload>,

    /// DelayAckInfo payload. Gated by `V2Flags::DELAYACKINFO`.
    pub delay_ack_info: Option<DelayAckInfoPayload>,

    /// AckOfAcks payload. Gated by `V2Flags::AOA`.
    pub ack_of_acks: Option<AckOfAcksPayload>,

    /// DataHeader payload. Gated by `V2Flags::DATA`.
    /// Must be paired with `data_body`.
    pub data_header: Option<DataHeader>,

    /// AckVector payload. Gated by `V2Flags::ACKVEC`.
    /// Mutually exclusive with `ack`.
    pub ack_vector: Option<AckVectorPayload>,

    /// DataBody payload. Gated by `V2Flags::DATA`.
    /// Must be paired with `data_header`.
    /// Always last in wire layout (consumes remaining bytes on decode).
    pub data_body: Option<DataBody>,
}

impl V2Packet {
    const NAME: &'static str = "RDP-UDP2 Packet";

    /// Compute flags from populated fields.
    ///
    /// Every flag [MS-RDPEUDP2] 2.2.1.1 defines announces a payload, so there
    /// is nothing to carry over from the caller's header: what is present
    /// determines the field completely.
    fn compute_flags(&self) -> V2Flags {
        let mut flags = V2Flags::empty();

        if self.ack.is_some() {
            flags |= V2Flags::ACK;
        }
        if self.overhead_size.is_some() {
            flags |= V2Flags::OVERHEADSIZE;
        }
        if self.delay_ack_info.is_some() {
            flags |= V2Flags::DELAYACKINFO;
        }
        if self.ack_of_acks.is_some() {
            flags |= V2Flags::AOA;
        }
        if self.data_header.is_some() {
            flags |= V2Flags::DATA;
        }
        if self.ack_vector.is_some() {
            flags |= V2Flags::ACKVEC;
        }

        flags
    }
}

impl Encode for V2Packet {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        // Validate invariants before writing anything
        if self.data_header.is_some() != self.data_body.is_some() {
            return Err(ironrdp_core::invalid_field_err!(
                "V2Packet",
                "DATA",
                "data_header and data_body must be both present or both absent"
            ));
        }
        if self.ack.is_some() && self.ack_vector.is_some() {
            return Err(ironrdp_core::invalid_field_err!(
                "V2Packet",
                "ACK/ACKVEC",
                "ACK and ACKVEC are mutually exclusive"
            ));
        }

        ironrdp_core::ensure_size!(in: dst, size: self.size());

        // Write header with auto-computed flags
        let header = V2Header {
            flags: self.compute_flags(),
            ..self.header
        };
        header.encode(dst)?;

        // Write payloads in spec-mandated order (MS-RDPEUDP2 Section 2.2.1)
        if let Some(ref ack) = self.ack {
            ack.encode(dst)?;
        }
        if let Some(ref overhead) = self.overhead_size {
            overhead.encode(dst)?;
        }
        if let Some(ref dai) = self.delay_ack_info {
            dai.encode(dst)?;
        }
        if let Some(ref aoa) = self.ack_of_acks {
            aoa.encode(dst)?;
        }
        if let Some(ref dh) = self.data_header {
            dh.encode(dst)?;
        }
        if let Some(ref av) = self.ack_vector {
            av.encode(dst)?;
        }
        if let Some(ref db) = self.data_body {
            db.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let mut total = self.header.size();
        if let Some(ref a) = self.ack {
            total += a.size();
        }
        if let Some(ref os) = self.overhead_size {
            total += os.size();
        }
        if let Some(ref dai) = self.delay_ack_info {
            total += dai.size();
        }
        if let Some(ref aoa) = self.ack_of_acks {
            total += aoa.size();
        }
        if let Some(ref dh) = self.data_header {
            total += dh.size();
        }
        if let Some(ref av) = self.ack_vector {
            total += av.size();
        }
        if let Some(ref db) = self.data_body {
            total += db.size();
        }
        total
    }
}

impl Decode<'_> for V2Packet {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        // V2Header::decode validates ACK/ACKVEC mutual exclusion
        let header = V2Header::decode(src)?;

        let ack = if header.flags.contains(V2Flags::ACK) {
            Some(AckPayload::decode(src)?)
        } else {
            None
        };

        let overhead_size = if header.flags.contains(V2Flags::OVERHEADSIZE) {
            Some(OverheadSizePayload::decode(src)?)
        } else {
            None
        };

        let delay_ack_info = if header.flags.contains(V2Flags::DELAYACKINFO) {
            Some(DelayAckInfoPayload::decode(src)?)
        } else {
            None
        };

        let ack_of_acks = if header.flags.contains(V2Flags::AOA) {
            Some(AckOfAcksPayload::decode(src)?)
        } else {
            None
        };

        let data_header = if header.flags.contains(V2Flags::DATA) {
            Some(DataHeader::decode(src)?)
        } else {
            None
        };

        let ack_vector = if header.flags.contains(V2Flags::ACKVEC) {
            Some(AckVectorPayload::decode(src)?)
        } else {
            None
        };

        // DataBody is always last: it consumes all remaining bytes
        let data_body = if header.flags.contains(V2Flags::DATA) {
            Some(DataBody::decode(src)?)
        } else {
            None
        };

        Ok(Self {
            header,
            ack,
            overhead_size,
            delay_ack_info,
            ack_of_acks,
            data_header,
            ack_vector,
            data_body,
        })
    }
}
