//! RDP-UDP reliable transport wire structures.
//!
//! The transport state machine lives outside this crate. These types encode the
//! common header, connection-initialization, acknowledgement, and source-data
//! structures used by reliable RDP-UDP versions 1 and 2.
//!
//! Defined in [\[MS-RDPEUDP\] 2.2.2.1, 2.2.2.4, 2.2.2.5, and 2.2.2.7].
//!
//! [\[MS-RDPEUDP\] 2.2.2.1, 2.2.2.4, 2.2.2.5, and 2.2.2.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeudp/2744a3ee-04fb-407b-a9e3-b3b2ded422b1

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
        /// The data payload is FEC encoded and follows an FEC payload header.
        const FEC = 0x0010;
        /// Congestion notification.
        const CN = 0x0020;
        /// Congestion-window reset.
        const CWR = 0x0040;
        /// Selective ACK option; currently unused by the protocol.
        const SACK_OPTION = 0x0080;
        /// ACK-of-ACK vector follows the ACK vector.
        const ACK_OF_ACKS = 0x0100;
        /// Requests lossy transport; this is unsupported by the reliable-only engine.
        const SYN_LOSSY = 0x0200;
        /// Indicates delayed acknowledgement.
        const ACK_DELAYED = 0x0400;
        /// A correlation ID follows SYN data.
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

/// Header and bytes of a reliable RDP-UDP source payload.
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
    /// Bytes delivered to the upper transport layer.
    pub data: Vec<u8>,
}

impl RdpUdpSourcePayload {
    const FIXED_PART_SIZE: usize = 4 /* snCoded */ + 4 /* snSourceStart */;
    const NAME: &'static str = "RdpUdpSourcePayload";
}

impl Encode for RdpUdpSourcePayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32_be(self.coded_sequence_number);
        dst.write_u32_be(self.source_sequence_number);
        dst.write_slice(&self.data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data.len()
    }
}

impl<'de> Decode<'de> for RdpUdpSourcePayload {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let coded_sequence_number = src.read_u32_be();
        let source_sequence_number = src.read_u32_be();
        let data = src.read_remaining().to_vec();

        Ok(Self {
            coded_sequence_number,
            source_sequence_number,
            data,
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
}
