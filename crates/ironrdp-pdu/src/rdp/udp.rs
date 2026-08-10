//! RDP-UDP reliable transport wire structures.
//!
//! The transport state machine lives outside this crate. These types encode the
//! common header, connection-initialization, acknowledgement, and data payload
//! headers used by reliable RDP-UDP.
//!
//! Defined in [\[MS-RDPEUDP\] 2.2.2.1 through 2.2.2.9].
//!
//! [\[MS-RDPEUDP\] 2.2.2.1 through 2.2.2.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1

use bitflags::bitflags;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err, read_padding,
};

/// Minimum MTU accepted during reliable RDP-UDP negotiation.
pub const MIN_MTU: u16 = 1132;
/// Maximum MTU accepted during reliable RDP-UDP negotiation.
pub const MAX_MTU: u16 = 1232;
/// Maximum encoded ACK vector size.
pub const MAX_ACK_VECTOR_SIZE: usize = 2048;

bitflags! {
    /// Flags in an [`RdpUdpFecHeader`].
    ///
    /// Defined in [\[MS-RDPEUDP\] 2.2.2.1].
    ///
    /// [\[MS-RDPEUDP\] 2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    pub struct RdpUdpFlags: u16 {
        /// Connection initialization.
        const SYN = 0x0001;
        /// Connection termination; currently unused by the protocol.
        const FIN = 0x0002;
        /// An ACK vector follows the common header.
        const ACK = 0x0004;
        /// A data payload follows the ACK structures.
        const DATA = 0x0008;
        /// An FEC payload header follows the common header.
        const FEC = 0x0010;
        /// Congestion notification.
        const CN = 0x0020;
        /// Congestion-window reset.
        const CWR = 0x0040;
        /// Selective ACK option; currently unused by the protocol.
        const SACK_OPTION = 0x0080;
        /// An ACK-of-ACK vector header follows the ACK vector.
        const ACK_OF_ACKS = 0x0100;
        /// Requests lossy transport.
        const SYN_LOSSY = 0x0200;
        /// Indicates delayed acknowledgement.
        const ACK_DELAYED = 0x0400;
        /// A correlation ID payload follows SYN data.
        const CORRELATION_ID = 0x0800;
        /// Extended SYN data follows SYN data.
        const SYN_EX = 0x1000;
    }
}

/// Common RDP-UDP datagram header.
///
/// All fields are transmitted in network byte order.
///
/// Defined in [\[MS-RDPEUDP\] 2.2.2.1].
///
/// [\[MS-RDPEUDP\] 2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdpUdpFecHeader {
    /// Highest source sequence number observed from the peer.
    pub source_ack: u32,
    /// Peer-facing receive window in datagrams.
    pub receive_window_size: u16,
    /// Present optional structures and connection flags.
    pub flags: RdpUdpFlags,
}

impl RdpUdpFecHeader {
    const FIXED_PART_SIZE: usize = 4 /* snSourceAck */ + 2 /* uReceiveWindowSize */ + 2 /* uFlags */;
    const NAME: &'static str = "RdpUdpFecHeader";
}

impl Encode for RdpUdpFecHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32_be(self.source_ack);
        dst.write_u16_be(self.receive_window_size);
        dst.write_u16_be(self.flags.bits());
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for RdpUdpFecHeader {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let source_ack = src.read_u32_be();
        let receive_window_size = src.read_u16_be();
        let flags = RdpUdpFlags::from_bits_truncate(src.read_u16_be());

        Ok(Self {
            source_ack,
            receive_window_size,
            flags,
        })
    }
}

/// FEC payload metadata.
///
/// Defined in [\[MS-RDPEUDP\] 2.2.2.2].
///
/// [\[MS-RDPEUDP\] 2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdpUdpFecPayloadHeader {
    /// Sequence number of this coded packet.
    pub coded_sequence_number: u32,
    /// First source sequence number contained in the FEC payload.
    pub source_sequence_number: u32,
    /// Source-packet range covered by the FEC payload.
    pub range: u8,
    /// FEC engine index.
    pub fec_index: u8,
    /// Padding bytes supplied by the FEC engine.
    pub padding: [u8; 2],
}

impl RdpUdpFecPayloadHeader {
    const FIXED_PART_SIZE: usize = 4 /* snCoded */
        + 4 /* snSourceStart */
        + 1 /* uRange */
        + 1 /* uFecIndex */
        + 2 /* uPadding */;
    const NAME: &'static str = "RdpUdpFecPayloadHeader";
}

impl Encode for RdpUdpFecPayloadHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32_be(self.coded_sequence_number);
        dst.write_u32_be(self.source_sequence_number);
        dst.write_u8(self.range);
        dst.write_u8(self.fec_index);
        dst.write_array(self.padding);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for RdpUdpFecPayloadHeader {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self {
            coded_sequence_number: src.read_u32_be(),
            source_sequence_number: src.read_u32_be(),
            range: src.read_u8(),
            fec_index: src.read_u8(),
            padding: src.read_array(),
        })
    }
}

/// Length prefix for a recovered FEC payload.
///
/// Defined in [\[MS-RDPEUDP\] 2.2.2.3].
///
/// [\[MS-RDPEUDP\] 2.2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdpUdpPayloadPrefix {
    /// Length of the recovered payload.
    pub payload_size: u16,
}

impl RdpUdpPayloadPrefix {
    const FIXED_PART_SIZE: usize = 2 /* cbPayloadSize */;
    const NAME: &'static str = "RdpUdpPayloadPrefix";
}

impl Encode for RdpUdpPayloadPrefix {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u16_be(self.payload_size);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for RdpUdpPayloadPrefix {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self {
            payload_size: src.read_u16_be(),
        })
    }
}

/// Parameters exchanged during the RDP-UDP SYN handshake.
///
/// Defined in [\[MS-RDPEUDP\] 2.2.2.5].
///
/// [\[MS-RDPEUDP\] 2.2.2.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdpUdpSynData {
    /// First coded and source sequence number used by the sender.
    pub initial_sequence_number: u32,
    /// Largest source payload the sender can transmit.
    pub upstream_mtu: u16,
    /// Largest source payload the sender can receive.
    pub downstream_mtu: u16,
}

impl RdpUdpSynData {
    const FIXED_PART_SIZE: usize = 4 /* snInitialSequenceNumber */ + 2 /* uUpStreamMtu */ + 2 /* uDownStreamMtu */;
    const NAME: &'static str = "RdpUdpSynData";

    fn has_valid_mtu(mtu: u16) -> bool {
        (MIN_MTU..=MAX_MTU).contains(&mtu)
    }
}

impl Encode for RdpUdpSynData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !Self::has_valid_mtu(self.upstream_mtu) {
            return Err(invalid_field_err!(
                "uUpStreamMtu",
                "must be between 1132 and 1232 bytes"
            ));
        }
        if !Self::has_valid_mtu(self.downstream_mtu) {
            return Err(invalid_field_err!(
                "uDownStreamMtu",
                "must be between 1132 and 1232 bytes"
            ));
        }
        ensure_fixed_part_size!(in: dst);
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

impl<'de> Decode<'de> for RdpUdpSynData {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let initial_sequence_number = src.read_u32_be();
        let upstream_mtu = src.read_u16_be();
        let downstream_mtu = src.read_u16_be();
        if !Self::has_valid_mtu(upstream_mtu) {
            return Err(invalid_field_err!(
                "uUpStreamMtu",
                "must be between 1132 and 1232 bytes"
            ));
        }
        if !Self::has_valid_mtu(downstream_mtu) {
            return Err(invalid_field_err!(
                "uDownStreamMtu",
                "must be between 1132 and 1232 bytes"
            ));
        }

        Ok(Self {
            initial_sequence_number,
            upstream_mtu,
            downstream_mtu,
        })
    }
}

/// ACK-vector reset metadata.
///
/// Defined in [\[MS-RDPEUDP\] 2.2.2.6].
///
/// [\[MS-RDPEUDP\] 2.2.2.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdpUdpAckOfAckVectorHeader {
    /// Sequence number that resets ACK-vector encoding state.
    pub reset_sequence_number: u32,
}

impl RdpUdpAckOfAckVectorHeader {
    const FIXED_PART_SIZE: usize = 4 /* snResetSeqNum */;
    const NAME: &'static str = "RdpUdpAckOfAckVectorHeader";
}

impl Encode for RdpUdpAckOfAckVectorHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32_be(self.reset_sequence_number);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for RdpUdpAckOfAckVectorHeader {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self {
            reset_sequence_number: src.read_u32_be(),
        })
    }
}

/// RDP-UDP ACK vector with DWORD alignment padding.
///
/// Defined in [\[MS-RDPEUDP\] 2.2.2.7].
///
/// [\[MS-RDPEUDP\] 2.2.2.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdpUdpAckVector {
    /// Run-length encoded ACK-vector elements.
    pub encoded: Vec<u8>,
}

impl RdpUdpAckVector {
    const FIXED_PART_SIZE: usize = 2 /* uAckVectorSize */;
    const NAME: &'static str = "RdpUdpAckVector";

    fn padding_size(encoded_len: usize) -> usize {
        (4 - ((Self::FIXED_PART_SIZE + encoded_len) % 4)) % 4
    }
}

impl Encode for RdpUdpAckVector {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.encoded.len() > MAX_ACK_VECTOR_SIZE {
            return Err(invalid_field_err!("uAckVectorSize", "must not exceed 2048 bytes"));
        }

        let encoded_len: u16 = cast_length!("uAckVectorSize", self.encoded.len())?;
        ensure_size!(in: dst, size: self.size());
        dst.write_u16_be(encoded_len);
        dst.write_slice(&self.encoded);
        dst.write_slice(&[0; 3][..Self::padding_size(self.encoded.len())]);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.encoded.len() + Self::padding_size(self.encoded.len())
    }
}

impl<'de> Decode<'de> for RdpUdpAckVector {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::FIXED_PART_SIZE);
        let encoded_len = usize::from(src.read_u16_be());
        if encoded_len > MAX_ACK_VECTOR_SIZE {
            return Err(invalid_field_err!("uAckVectorSize", "must not exceed 2048 bytes"));
        }

        let padding_size = Self::padding_size(encoded_len);
        ensure_size!(in: src, size: encoded_len + padding_size);
        let encoded = src.read_slice(encoded_len).to_vec();
        read_padding(src, padding_size);

        Ok(Self { encoded })
    }
}

/// Header of a reliable RDP-UDP source payload.
///
/// Defined in [\[MS-RDPEUDP\] 2.2.2.4].
///
/// [\[MS-RDPEUDP\] 2.2.2.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdpUdpSourcePayload {
    /// Sequence number of this transmitted datagram.
    pub coded_sequence_number: u32,
    /// Sequence number of the encapsulated source payload.
    pub source_sequence_number: u32,
}

impl RdpUdpSourcePayload {
    const FIXED_PART_SIZE: usize = 4 /* snCoded */ + 4 /* snSourceStart */;
    const NAME: &'static str = "RdpUdpSourcePayload";
}

impl Encode for RdpUdpSourcePayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32_be(self.coded_sequence_number);
        dst.write_u32_be(self.source_sequence_number);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for RdpUdpSourcePayload {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let coded_sequence_number = src.read_u32_be();
        let source_sequence_number = src.read_u32_be();

        Ok(Self {
            coded_sequence_number,
            source_sequence_number,
        })
    }
}

/// Correlation ID negotiated by the RDP-UDP endpoint.
///
/// Defined in [\[MS-RDPEUDP\] 2.2.2.8].
///
/// [\[MS-RDPEUDP\] 2.2.2.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdpUdpCorrelationId {
    /// Connection correlation identifier in wire byte order.
    pub correlation_id: [u8; 16],
}

impl RdpUdpCorrelationId {
    const FIXED_PART_SIZE: usize = 16 /* uCorrelationId */ + 16 /* uReserved */;
    const NAME: &'static str = "RdpUdpCorrelationId";
}

impl Encode for RdpUdpCorrelationId {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_array(self.correlation_id);
        dst.write_array([0; 16]);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for RdpUdpCorrelationId {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let correlation_id = src.read_array();
        let reserved = src.read_array::<16>();
        if reserved != [0; 16] {
            return Err(invalid_field_err!("uReserved", "must be all zeroes"));
        }

        Ok(Self { correlation_id })
    }
}

bitflags! {
    /// Extended SYN-data flags.
    ///
    /// Defined in [\[MS-RDPEUDP\] 2.2.2.9].
    ///
    /// [\[MS-RDPEUDP\] 2.2.2.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    pub struct RdpUdpSynDataExFlags: u16 {
        /// The protocol version field is valid.
        const VERSION_INFO_VALID = 0x0001;
    }
}

/// RDP-UDP protocol version 3.
///
/// Version 3 uses the data transfer messages defined by MS-RDPEUDP2.
pub const RDPUDP_PROTOCOL_VERSION_3: u16 = 0x0101;

/// Extended RDP-UDP SYN parameters.
///
/// Defined in [\[MS-RDPEUDP\] 2.2.2.9].
///
/// [\[MS-RDPEUDP\] 2.2.2.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct RdpUdpSynDataEx {
    /// Extended SYN options.
    pub flags: RdpUdpSynDataExFlags,
    /// Highest supported RDP-UDP protocol version.
    pub protocol_version: u16,
    /// Required cookie hash for protocol version 3, absent for all other versions.
    pub cookie_hash: Option<[u8; 32]>,
}

impl RdpUdpSynDataEx {
    const FIXED_PART_SIZE: usize = 2 /* uSynExFlags */ + 2 /* uUdpVer */;
    const NAME: &'static str = "RdpUdpSynDataEx";

    fn has_valid_cookie_hash(&self) -> bool {
        (self.protocol_version == RDPUDP_PROTOCOL_VERSION_3) == self.cookie_hash.is_some()
    }
}

impl Encode for RdpUdpSynDataEx {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !self.has_valid_cookie_hash() {
            return Err(invalid_field_err!(
                "cookieHash",
                "must be present only for RDPUDP_PROTOCOL_VERSION_3"
            ));
        }

        ensure_size!(in: dst, size: self.size());
        dst.write_u16_be(self.flags.bits());
        dst.write_u16_be(self.protocol_version);
        if let Some(cookie_hash) = self.cookie_hash {
            dst.write_array(cookie_hash);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.cookie_hash.map_or(0, |hash| hash.len())
    }
}

impl<'de> Decode<'de> for RdpUdpSynDataEx {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let flags = RdpUdpSynDataExFlags::from_bits_truncate(src.read_u16_be());
        let protocol_version = src.read_u16_be();
        let cookie_hash = if protocol_version == RDPUDP_PROTOCOL_VERSION_3 {
            ensure_size!(in: src, size: 32);
            Some(src.read_array())
        } else {
            None
        };

        Ok(Self {
            flags,
            protocol_version,
            cookie_hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syn_packet_fields_use_network_byte_order() {
        let header = RdpUdpFecHeader {
            source_ack: 0xFFFF_FFFF,
            receive_window_size: 1024,
            flags: RdpUdpFlags::SYN | RdpUdpFlags::SYN_EX,
        };
        let encoded = ironrdp_core::encode_vec(&header).unwrap();
        assert_eq!(encoded.as_slice(), &[0xFF, 0xFF, 0xFF, 0xFF, 0x04, 0x00, 0x10, 0x01]);
        assert_eq!(ironrdp_core::decode::<RdpUdpFecHeader>(&encoded).unwrap(), header);
    }

    #[test]
    fn accepts_documented_and_unknown_flags() {
        let encoded = [
            0xFF, 0xFF, 0xFF, 0xFF, // snSourceAck
            0x04, 0x00, // uReceiveWindowSize
            0x80, 0x84, // ACK | SACK_OPTION | unknown flag
        ];
        let header = ironrdp_core::decode::<RdpUdpFecHeader>(&encoded).unwrap();
        assert_eq!(header.flags, RdpUdpFlags::ACK | RdpUdpFlags::SACK_OPTION);
    }

    #[test]
    fn ack_vector_round_trip_and_padding() {
        for encoded in [
            vec![],
            vec![0x04],
            vec![0x04, 0x05],
            vec![0x04, 0x05, 0x06],
            vec![0x04; 2048],
        ] {
            let vector = RdpUdpAckVector { encoded };
            let encoded = ironrdp_core::encode_vec(&vector).unwrap();
            assert_eq!(ironrdp_core::decode::<RdpUdpAckVector>(&encoded).unwrap(), vector);
        }
    }

    #[test]
    fn ack_vector_ignores_non_zero_padding() {
        let encoded = [0x00, 0x01, 0x04, 0xFF];
        assert_eq!(
            ironrdp_core::decode::<RdpUdpAckVector>(&encoded).unwrap(),
            RdpUdpAckVector { encoded: vec![0x04] }
        );
    }

    #[test]
    fn accepts_mtu_boundaries() {
        for mtu in [MIN_MTU, MAX_MTU] {
            let syn = RdpUdpSynData {
                initial_sequence_number: 1,
                upstream_mtu: mtu,
                downstream_mtu: mtu,
            };
            let encoded = ironrdp_core::encode_vec(&syn).unwrap();
            assert_eq!(ironrdp_core::decode::<RdpUdpSynData>(&encoded).unwrap(), syn);
        }
    }

    #[test]
    fn rejects_invalid_mtu_and_ack_vector_size() {
        let invalid_mtu = [0, 0, 0, 1, 0x04, 0x6B, 0x04, 0xD0];
        assert!(ironrdp_core::decode::<RdpUdpSynData>(&invalid_mtu).is_err());

        let invalid_ack_size = [0x08, 0x01];
        assert!(ironrdp_core::decode::<RdpUdpAckVector>(&invalid_ack_size).is_err());
    }

    #[test]
    fn supports_optional_udp_header_codecs() {
        let fec = RdpUdpFecPayloadHeader {
            coded_sequence_number: 1,
            source_sequence_number: 2,
            range: 3,
            fec_index: 4,
            padding: [5, 6],
        };
        let prefix = RdpUdpPayloadPrefix { payload_size: 1232 };
        let ack_of_acks = RdpUdpAckOfAckVectorHeader {
            reset_sequence_number: 7,
        };
        let correlation = RdpUdpCorrelationId {
            correlation_id: [8; 16],
        };
        let syn_ex_v2 = RdpUdpSynDataEx {
            flags: RdpUdpSynDataExFlags::VERSION_INFO_VALID,
            protocol_version: 2,
            cookie_hash: None,
        };
        let syn_ex_v3 = RdpUdpSynDataEx {
            flags: RdpUdpSynDataExFlags::VERSION_INFO_VALID,
            protocol_version: RDPUDP_PROTOCOL_VERSION_3,
            cookie_hash: Some([9; 32]),
        };

        let encoded = ironrdp_core::encode_vec(&fec).unwrap();
        assert_eq!(ironrdp_core::decode::<RdpUdpFecPayloadHeader>(&encoded).unwrap(), fec);
        let encoded = ironrdp_core::encode_vec(&prefix).unwrap();
        assert_eq!(ironrdp_core::decode::<RdpUdpPayloadPrefix>(&encoded).unwrap(), prefix);
        let encoded = ironrdp_core::encode_vec(&ack_of_acks).unwrap();
        assert_eq!(
            ironrdp_core::decode::<RdpUdpAckOfAckVectorHeader>(&encoded).unwrap(),
            ack_of_acks
        );
        let encoded = ironrdp_core::encode_vec(&correlation).unwrap();
        assert_eq!(
            ironrdp_core::decode::<RdpUdpCorrelationId>(&encoded).unwrap(),
            correlation
        );
        let encoded = ironrdp_core::encode_vec(&syn_ex_v2).unwrap();
        assert_eq!(ironrdp_core::decode::<RdpUdpSynDataEx>(&encoded).unwrap(), syn_ex_v2);
        let encoded = ironrdp_core::encode_vec(&syn_ex_v3).unwrap();
        assert_eq!(ironrdp_core::decode::<RdpUdpSynDataEx>(&encoded).unwrap(), syn_ex_v3);
    }

    #[test]
    fn source_payload_header_preserves_following_payload() {
        let encoded = [
            0, 0, 0, 1, // snCoded
            0, 0, 0, 2, // snSourceStart
            3, 4, 5, // source payload
        ];
        let mut cursor = ReadCursor::new(&encoded);
        assert_eq!(
            RdpUdpSourcePayload::decode(&mut cursor).unwrap(),
            RdpUdpSourcePayload {
                coded_sequence_number: 1,
                source_sequence_number: 2,
            }
        );
        assert_eq!(cursor.remaining(), &[3, 4, 5]);
    }

    #[test]
    fn rejects_invalid_optional_udp_header_fields() {
        let correlation = [0; 32];
        let mut invalid_correlation = correlation;
        invalid_correlation[16] = 1;
        assert!(ironrdp_core::decode::<RdpUdpCorrelationId>(&invalid_correlation).is_err());

        let syn_ex = RdpUdpSynDataEx {
            flags: RdpUdpSynDataExFlags::VERSION_INFO_VALID,
            protocol_version: RDPUDP_PROTOCOL_VERSION_3,
            cookie_hash: None,
        };
        assert!(ironrdp_core::encode_vec(&syn_ex).is_err());
        assert!(ironrdp_core::decode::<RdpUdpSynDataEx>(&[0, 1, 1, 1]).is_err());
    }
}
