//! SYN-related payloads for the v1 three-way handshake.
//!
//! `RDPUDP_SYNDATA_PAYLOAD` (MS-RDPEUDP Section 2.2.2.5) and
//! `RDPUDP_SYNDATAEX_PAYLOAD` (MS-RDPEUDP Section 2.2.2.9).

use bitflags::bitflags;
use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

// -- MTU constants per MS-RDPEUDP Section 2.2.2.5 --

/// Minimum allowed MTU value.
pub const MTU_MIN: u16 = 1132;

/// Maximum allowed MTU value.
pub const MTU_MAX: u16 = 1232;

// -- RDPUDP_SYNDATA_PAYLOAD --

/// Connection initialization parameters exchanged in SYN and SYN+ACK datagrams.
///
/// MS-RDPEUDP Section 2.2.2.5.
/// Wire layout: `snInitialSequenceNumber(4)` + `uUpStreamMtu(2)` + `uDownStreamMtu(2)` = 8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynDataPayload {
    /// Starting sequence number (random, analogous to TCP ISN per RFC 1948).
    pub initial_sequence_number: u32,

    /// Maximum datagram size this endpoint can generate.
    /// Must be in `1132..=1232`.
    pub upstream_mtu: u16,

    /// Maximum datagram size this endpoint can accept.
    /// Must be in `1132..=1232`.
    pub downstream_mtu: u16,
}

/// Generates MTUs inside the range the decoder accepts.
///
/// The derived implementation would draw `uUpStreamMtu` and `uDownStreamMtu`
/// from the whole of `u16`, and 2.2.2.5 constrains both to 1132..=1232. Almost
/// every generated SYN would then be rejected by its own decoder, so the fuzzer
/// would spend its budget on the validation branch instead of reaching the
/// handshake behind it.
#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for SynDataPayload {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            initial_sequence_number: u32::arbitrary(u)?,
            upstream_mtu: u.int_in_range(MTU_MIN..=MTU_MAX)?,
            downstream_mtu: u.int_in_range(MTU_MIN..=MTU_MAX)?,
        })
    }
}

impl SynDataPayload {
    const FIXED_PART_SIZE: usize = 4 + 2 + 2;
    const NAME: &'static str = "RDPUDP_SYNDATA_PAYLOAD";

    fn validate_mtu<T: ironrdp_core::InvalidFieldErr>(value: u16, field: &'static str) -> Result<(), T> {
        if !(MTU_MIN..=MTU_MAX).contains(&value) {
            return Err(ironrdp_core::invalid_field_err!(
                "RDPUDP_SYNDATA_PAYLOAD",
                field,
                "mtu must be in 1132..=1232"
            ));
        }
        Ok(())
    }
}

impl Encode for SynDataPayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);

        Self::validate_mtu(self.upstream_mtu, "uUpStreamMtu")?;
        Self::validate_mtu(self.downstream_mtu, "uDownStreamMtu")?;

        dst.write_u32_be(self.initial_sequence_number);
        dst.write_u16_be(self.upstream_mtu);
        dst.write_u16_be(self.downstream_mtu);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for SynDataPayload {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);

        let initial_sequence_number = src.read_u32_be();
        let upstream_mtu = src.read_u16_be();
        let downstream_mtu = src.read_u16_be();

        Self::validate_mtu(upstream_mtu, "uUpStreamMtu")?;
        Self::validate_mtu(downstream_mtu, "uDownStreamMtu")?;

        Ok(Self {
            initial_sequence_number,
            upstream_mtu,
            downstream_mtu,
        })
    }
}

// -- SYNDATAEX flags --

bitflags! {
    /// Flags in the RDPUDP_SYNDATAEX_PAYLOAD `uSynExFlags` field.
    ///
    /// MS-RDPEUDP Section 2.2.2.9.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    pub struct SynExFlags: u16 {
        /// The `uUdpVer` field indicates a supported protocol version.
        const VERSION_INFO_VALID = 0x0001;
    }
}

// -- UDP protocol versions --

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// RDPEUDP protocol version negotiated via SYNDATAEX.
///
/// MS-RDPEUDP Section 2.2.2.9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum UdpVersion {
    /// v1: min retransmit 500ms, min ACK delay 200ms.
    /// Data transfer uses the MS-RDPEUDP wire format.
    V1 = 0x0001,
    /// v2: min retransmit 300ms, min ACK delay 50ms.
    /// Data transfer still uses the MS-RDPEUDP wire format.
    V2 = 0x0002,
    /// v3: data transfer uses the MS-RDPEUDP2 wire format.
    /// The client's SYN carries a cookieHash binding it to the
    /// multitransport request.
    V3 = 0x0101,
}

impl UdpVersion {
    /// Try to parse a version from its wire representation.
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0x0001 => Some(Self::V1),
            0x0002 => Some(Self::V2),
            0x0101 => Some(Self::V3),
            _ => None,
        }
    }

    /// Returns the wire representation.
    pub fn as_u16(self) -> u16 {
        match self {
            Self::V1 => 0x0001,
            Self::V2 => 0x0002,
            Self::V3 => 0x0101,
        }
    }

    /// Whether this version selects the MS-RDPEUDP2 wire format for data
    /// transfer.
    ///
    /// Only version 3 does. The name of version 2 invites the opposite
    /// reading and it is wrong: 2.2.2.9 describes 0x0002 purely as a pair of
    /// shorter timeouts, and 0x0101 is the only row that mentions
    /// [MS-RDPEUDP2]. 1.3.2.2 states it from the other direction, that the
    /// MS-RDPEUDP data transfer messages "MUST be used only when the version
    /// negotiated in the UDP connection initialization phase is version 1 or
    /// version 2".
    pub fn uses_v2_wire_format(self) -> bool {
        matches!(self, Self::V3)
    }
}

// -- RDPUDP_SYNDATAEX_PAYLOAD --

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Extended SYN parameters for version negotiation.
///
/// MS-RDPEUDP Section 2.2.2.9.
/// Wire layout: `uSynExFlags(2)` + `uUdpVer(2)` + optional `cookieHash(32)`.
///
/// `cookieHash` is present only in a client→server SYN with v3 (`RDPUDP_PROTOCOL_VERSION_3`).
/// It contains the SHA-256 hash of the server's `securityCookie` from the
/// Initiate Multitransport Request PDU (MS-RDPBCGR Section 2.2.15.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynDataExPayload {
    /// Extended flags (must include `VERSION_INFO_VALID` for version negotiation).
    pub syn_ex_flags: SynExFlags,

    /// Protocol version advertised by this endpoint.
    pub udp_ver: UdpVersion,

    /// SHA-256 hash of securityCookie. Present only for v3 client→server SYN.
    /// Encoded as 8 × 4-byte big-endian unsigned integers.
    pub cookie_hash: Option<[u8; 32]>,
}

impl SynDataExPayload {
    const FIXED_PART_SIZE: usize = 2 + 2; // uSynExFlags + uUdpVer
    const COOKIE_HASH_SIZE: usize = 32;
    const NAME: &'static str = "RDPUDP_SYNDATAEX_PAYLOAD";

    /// Total encoded size including optional cookie hash.
    fn encoded_size(&self) -> usize {
        let base = Self::FIXED_PART_SIZE;
        if self.cookie_hash.is_some() {
            base + Self::COOKIE_HASH_SIZE
        } else {
            base
        }
    }

    fn decode_fixed_part(src: &mut ReadCursor<'_>) -> DecodeResult<(SynExFlags, UdpVersion)> {
        ironrdp_core::ensure_fixed_part_size!(in: src);

        let syn_ex_flags_raw = src.read_u16_be();
        let syn_ex_flags = SynExFlags::from_bits_truncate(syn_ex_flags_raw);

        let udp_ver_raw = src.read_u16_be();
        let udp_ver = UdpVersion::from_u16(udp_ver_raw).ok_or_else(|| {
            ironrdp_core::invalid_field_err!("RDPUDP_SYNDATAEX_PAYLOAD", "uUdpVer", "unknown protocol version")
        })?;

        Ok((syn_ex_flags, udp_ver))
    }

    /// Decodes as part of a v1 datagram whose direction is already known.
    ///
    /// `cookieHash` is carried only in a client-to-server SYN with version 3
    /// (MS-RDPEUDP 2.2.2.9); a v3 SYN+ACK MUST NOT carry one. SYN and SYN+ACK
    /// datagrams are padded to the negotiated MTU (3.1.5.1.1, 3.1.5.1.3), so
    /// the plain [`Decode`] impl's remaining-length heuristic misreads that
    /// padding as a hash on a SYN+ACK. This reads exactly the bytes the
    /// direction says should be there, nothing inferred from what remains.
    pub(crate) fn decode_directional(src: &mut ReadCursor<'_>, is_client_syn: bool) -> DecodeResult<Self> {
        let (syn_ex_flags, udp_ver) = Self::decode_fixed_part(src)?;

        let cookie_hash = if is_client_syn && udp_ver == UdpVersion::V3 {
            ironrdp_core::ensure_size!(in: src, size: Self::COOKIE_HASH_SIZE);
            Some(src.read_array::<32>())
        } else {
            None
        };

        Ok(Self {
            syn_ex_flags,
            udp_ver,
            cookie_hash,
        })
    }
}

impl Encode for SynDataExPayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        // The decoder reads the cookie hash only for version 3, so a hash
        // carried alongside any other version would be written and never read
        // back. Reject it rather than drop it silently: a caller that set the
        // field meant it, and the combination cannot be put on the wire.
        if self.cookie_hash.is_some() && self.udp_ver != UdpVersion::V3 {
            return Err(ironrdp_core::invalid_field_err!(
                Self::NAME,
                "cookieHash",
                "is only carried when uUdpVer is version 3"
            ));
        }

        ironrdp_core::ensure_size!(in: dst, size: self.encoded_size());

        dst.write_u16_be(self.syn_ex_flags.bits());
        dst.write_u16_be(self.udp_ver.as_u16());

        if let Some(hash) = &self.cookie_hash {
            dst.write_slice(hash);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.encoded_size()
    }
}

impl Decode<'_> for SynDataExPayload {
    /// Decodes a standalone, exact-length `RDPUDP_SYNDATAEX_PAYLOAD` buffer.
    ///
    /// This infers `cookieHash` presence from the remaining buffer length,
    /// which is correct only when the buffer holds exactly this structure and
    /// nothing after it. It must not be used on bytes carved out of a padded
    /// v1 datagram; use `SynDataExPayload::decode_directional` there.
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let (syn_ex_flags, udp_ver) = Self::decode_fixed_part(src)?;

        // cookieHash is present only for v3 and when there are enough remaining bytes.
        // Per spec: MUST be present in client→server SYN with v3, MUST NOT be present otherwise.
        let cookie_hash = if udp_ver == UdpVersion::V3 && src.len() >= Self::COOKIE_HASH_SIZE {
            Some(src.read_array::<32>())
        } else {
            None
        };

        Ok(Self {
            syn_ex_flags,
            udp_ver,
            cookie_hash,
        })
    }
}
