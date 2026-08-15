//! RDPEUDP PDU definitions.
//!
//! V1 handshake format, MS-RDPEUDP section 2.2.
//!
//! These structures carry the three-way handshake (SYN, SYN+ACK, ACK) that
//! opens an RDP-UDP connection. They are used whatever protocol version the
//! endpoints settle on, because the version is what the handshake negotiates.
//!
//! Everything here is big-endian: section 2.2 says "all of the messages
//! written to the network or read from the network MUST be in network byte
//! order", and the field diagrams number bits most significant first. The
//! MS-RDPEUDP2 data transfer that a version 3 handshake leads to takes the
//! opposite convention on both counts.

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

// ── V1 Handshake modules ──

pub mod v1_ack;
pub mod v1_flags;
pub mod v1_header;
pub mod v1_syn;

// ── V1 re-exports ──

pub use v1_ack::{CorrelationIdPayload, V1AckOfAcksHeader, V1AckVectorElement, V1AckVectorHeader, VectorElementState};
pub use v1_flags::V1Flags;
pub use v1_header::FecHeader;
pub use v1_syn::{MTU_MAX, MTU_MIN, SynDataExPayload, SynDataPayload, SynExFlags, UdpVersion};

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
