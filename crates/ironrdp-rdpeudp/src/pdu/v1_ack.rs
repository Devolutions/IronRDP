//! V1 ACK-related structures.
//!
//! `RDPUDP_ACK_VECTOR_HEADER` (MS-RDPEUDP Section 2.2.2.7) and
//! `RDPUDP_ACK_OF_ACKVECTOR_HEADER` (MS-RDPEUDP Section 2.2.2.6).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

// -- ACK Vector Element --

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// State of a run of datagrams in the ACK vector.
///
/// [MS-RDPEUDP] 2.2.1.1. The two reserved values are defined by the
/// specification and unused by it, so they are represented rather than
/// rejected: a peer that sends one is speaking the protocol as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorElementState {
    /// The datagrams in this run were received.
    DatagramReceived,
    /// Reserved by the specification, not used.
    Reserved1,
    /// Reserved by the specification, not used.
    Reserved2,
    /// The datagrams in this run have not been received yet.
    DatagramNotYetReceived,
}

impl VectorElementState {
    fn to_bits(self) -> u8 {
        match self {
            Self::DatagramReceived => 0,
            Self::Reserved1 => 1,
            Self::Reserved2 => 2,
            Self::DatagramNotYetReceived => 3,
        }
    }

    fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::DatagramReceived,
            1 => Self::Reserved1,
            2 => Self::Reserved2,
            _ => Self::DatagramNotYetReceived,
        }
    }

    /// Whether this state means the datagrams arrived.
    #[must_use]
    pub fn is_received(self) -> bool {
        matches!(self, Self::DatagramReceived)
    }
}

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// One run in the ACK vector.
///
/// [MS-RDPEUDP] 2.2.2.7.1: the two most significant bits carry the
/// [`VectorElementState`] and the six least significant bits carry the length
/// of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V1AckVectorElement {
    /// State shared by every datagram in this run.
    pub state: VectorElementState,

    /// Number of consecutive datagrams in this state (0..=63).
    pub length: u8,
}

impl V1AckVectorElement {
    /// Longest run a single element can express: the length field is six bits.
    pub const MAX_LENGTH: u8 = 0x3F;

    /// Encode to a single byte: state in bits 6 and 7, run length in bits 0
    /// through 5.
    fn to_byte(self) -> u8 {
        (self.state.to_bits() << 6) | (self.length & Self::MAX_LENGTH)
    }

    /// Decode from a single byte.
    fn from_byte(byte: u8) -> Self {
        Self {
            state: VectorElementState::from_bits(byte >> 6),
            length: byte & Self::MAX_LENGTH,
        }
    }
}

// -- RDPUDP_ACK_VECTOR_HEADER --

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// ACK vector describing the state of the receiver's packet queue.
///
/// [MS-RDPEUDP] 2.2.2.7.
/// Wire layout: `uAckVectorSize(2)` + `AckVectorElement[](variable)` +
/// `Padding(variable)`.
///
/// The elements are run-length encoded, each carrying a state and the number
/// of consecutive datagrams sharing it. The structure is then padded so it
/// ends on a DWORD boundary, which 2.2.2.7 requires and the section 4.2.1
/// capture shows (`00 01 04 00`: two size bytes, one element, one pad byte).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1AckVectorHeader {
    /// RLE-encoded state of packets in the receiver queue.
    pub elements: Vec<V1AckVectorElement>,
}

impl V1AckVectorHeader {
    const FIXED_PART_SIZE: usize = 2; // uAckVectorSize
    const NAME: &'static str = "RDPUDP_ACK_VECTOR_HEADER";

    /// Bytes of padding needed so the structure ends on a DWORD boundary.
    fn padding_len(element_count: usize) -> usize {
        (Self::FIXED_PART_SIZE + element_count).next_multiple_of(4) - Self::FIXED_PART_SIZE - element_count
    }
}

impl Encode for V1AckVectorHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let count: u16 = ironrdp_core::cast_length!("RDPUDP_ACK_VECTOR_HEADER", "uAckVectorSize", self.elements.len())?;

        ironrdp_core::ensure_size!(in: dst, size: self.size());

        dst.write_u16_be(count);
        for element in &self.elements {
            dst.write_u8(element.to_byte());
        }
        ironrdp_core::write_padding!(dst, Self::padding_len(self.elements.len()));

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.elements.len() + Self::padding_len(self.elements.len())
    }
}

impl Decode<'_> for V1AckVectorHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);

        let count = src.read_u16_be();
        let count_usize = usize::from(count);

        // The padding is part of the structure, so it has to be present before
        // we start consuming: ensuring only the elements would let a truncated
        // datagram advance the cursor past the end while skipping it.
        let padding = Self::padding_len(count_usize);
        ironrdp_core::ensure_size!(in: src, size: count_usize + padding);

        let mut elements = Vec::with_capacity(count_usize);
        for _ in 0..count_usize {
            elements.push(V1AckVectorElement::from_byte(src.read_u8()));
        }
        ironrdp_core::read_padding!(src, padding);

        Ok(Self { elements })
    }
}

// -- RDPUDP_ACK_OF_ACKVECTOR_HEADER --

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Resets the starting position of ACK vector encoding at the receiver.
///
/// MS-RDPEUDP Section 2.2.2.6.
/// Wire layout: `snResetSeqNum(4)` = 4 bytes.
///
/// Sent after approximately every 20 packets. The receiver generates
/// ACK vectors only for sequence numbers greater than `reset_seq_num`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V1AckOfAcksHeader {
    /// Sequence number to reset the ACK vector base to.
    /// The sender populates this with the greatest cumulative ACK
    /// it has received and processed.
    pub reset_seq_num: u32,
}

impl V1AckOfAcksHeader {
    const FIXED_PART_SIZE: usize = 4;
    const NAME: &'static str = "RDPUDP_ACK_OF_ACKVECTOR_HEADER";
}

impl Encode for V1AckOfAcksHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);
        dst.write_u32_be(self.reset_seq_num);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for V1AckOfAcksHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);
        let reset_seq_num = src.read_u32_be();
        Ok(Self { reset_seq_num })
    }
}

// -- RDPUDP_CORRELATION_ID_PAYLOAD --

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Correlation ID for diagnostic binding between TCP and UDP connections.
///
/// [MS-RDPEUDP] 2.2.2.8.
/// Wire layout: `uCorrelationId(16)` + `uReserved(16)` = 32 bytes.
///
/// `uReserved` is sixteen zero bytes and is not represented as a field: 3.1.5.1.1
/// requires it to be all zeros on send, so there is nothing for a caller to
/// choose. It is written and skipped by the codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrelationIdPayload {
    /// A 16-byte identifier that correlates the UDP connection
    /// with its parent TCP connection for diagnostic purposes.
    pub correlation_id: [u8; 16],
}

impl CorrelationIdPayload {
    const CORRELATION_ID_SIZE: usize = 16;
    const RESERVED_SIZE: usize = 16;
    const FIXED_PART_SIZE: usize = Self::CORRELATION_ID_SIZE + Self::RESERVED_SIZE;
    const NAME: &'static str = "RDPUDP_CORRELATION_ID_PAYLOAD";
}

impl Encode for CorrelationIdPayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);
        dst.write_slice(&self.correlation_id);
        ironrdp_core::write_padding!(dst, Self::RESERVED_SIZE);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for CorrelationIdPayload {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);
        let correlation_id = src.read_array::<{ Self::CORRELATION_ID_SIZE }>();
        ironrdp_core::read_padding!(src, Self::RESERVED_SIZE);
        Ok(Self { correlation_id })
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_core::{decode, encode_vec};

    use super::*;

    // -- V1AckVectorElement tests --
    //
    // Byte layout is [MS-RDPEUDP] 2.2.2.7.1: state in bits 6 and 7, run length
    // in bits 0 through 5.

    #[test]
    fn ack_vector_element_received() {
        let elem = V1AckVectorElement {
            state: VectorElementState::DatagramReceived,
            length: 42,
        };

        // DATAGRAM_RECEIVED is 0, so a received run is just its length.
        assert_eq!(elem.to_byte(), 42);
        assert_eq!(V1AckVectorElement::from_byte(42), elem);
    }

    #[test]
    fn ack_vector_element_not_received() {
        let elem = V1AckVectorElement {
            state: VectorElementState::DatagramNotYetReceived,
            length: 3,
        };

        // DATAGRAM_NOT_YET_RECEIVED is 3, which lands in bits 6 and 7.
        assert_eq!(elem.to_byte(), 0xC3);
        assert_eq!(V1AckVectorElement::from_byte(0xC3), elem);
    }

    #[test]
    fn ack_vector_element_reserved_states_survive_a_round_trip() {
        for (state, bits) in [
            (VectorElementState::Reserved1, 0x40),
            (VectorElementState::Reserved2, 0x80),
        ] {
            let elem = V1AckVectorElement { state, length: 5 };
            assert_eq!(elem.to_byte(), bits | 5);
            assert_eq!(V1AckVectorElement::from_byte(bits | 5), elem);
        }
    }

    #[test]
    fn ack_vector_element_max_length() {
        let elem = V1AckVectorElement {
            state: VectorElementState::DatagramNotYetReceived,
            length: V1AckVectorElement::MAX_LENGTH,
        };

        assert_eq!(elem.to_byte(), 0xFF);
        assert_eq!(V1AckVectorElement::from_byte(0xFF), elem);
    }

    /// The run length is six bits, so a longer run cannot be expressed and
    /// must not silently overflow into the state bits.
    #[test]
    fn ack_vector_element_length_is_masked_not_overflowed() {
        let elem = V1AckVectorElement {
            state: VectorElementState::DatagramReceived,
            length: 64,
        };

        assert_eq!(elem.to_byte() >> 6, 0, "length overflowed into the state bits");
    }

    // -- V1AckVectorHeader tests --

    #[test]
    fn encode_ack_vector_empty() {
        let header = V1AckVectorHeader { elements: Vec::new() };
        let encoded = encode_vec(&header).expect("encode");

        // Two size bytes are not on a DWORD boundary, so two pad bytes follow.
        assert_eq!(encoded.as_slice(), &[0x00, 0x00, 0x00, 0x00]);
        assert_eq!(header.size(), 4);
    }

    #[test]
    fn encode_ack_vector_with_elements() {
        let header = V1AckVectorHeader {
            elements: vec![
                V1AckVectorElement {
                    state: VectorElementState::DatagramReceived,
                    length: 10,
                },
                V1AckVectorElement {
                    state: VectorElementState::DatagramNotYetReceived,
                    length: 2,
                },
                V1AckVectorElement {
                    state: VectorElementState::DatagramReceived,
                    length: 5,
                },
            ],
        };

        let encoded = encode_vec(&header).expect("encode");
        assert_eq!(
            encoded.as_slice(),
            &[
                0x00, 0x03, // uAckVectorSize = 3, network byte order
                10,   // DATAGRAM_RECEIVED, run of 10
                0xC2, // DATAGRAM_NOT_YET_RECEIVED, run of 2
                5,    // DATAGRAM_RECEIVED, run of 5
                0x00, 0x00, 0x00, // padding to a DWORD boundary
            ]
        );
        assert_eq!(header.size(), 8);
    }

    /// The section 4.2.1 capture, which is the authority for both the byte
    /// order and the padding.
    #[test]
    fn decode_ack_vector_from_the_spec_capture() {
        // From [MS-RDPEUDP] 4.2.1: uAckVectorSize 0x0001, one element 0x04,
        // then one pad byte.
        let bytes = [0x00, 0x01, 0x04, 0x00];

        let header: V1AckVectorHeader = decode(&bytes).expect("decode");
        assert_eq!(
            header.elements,
            vec![V1AckVectorElement {
                state: VectorElementState::DatagramReceived,
                length: 4,
            }],
            "the capture documents element 0x04 as DATAGRAM_RECEIVED with a run of 4"
        );

        assert_eq!(encode_vec(&header).expect("encode"), bytes);
    }

    #[test]
    fn ack_vector_roundtrip() {
        for count in [0usize, 1, 2, 3, 4, 5, 9] {
            let original = V1AckVectorHeader {
                elements: (0..count)
                    .map(|i| V1AckVectorElement {
                        state: if i % 2 == 0 {
                            VectorElementState::DatagramReceived
                        } else {
                            VectorElementState::DatagramNotYetReceived
                        },
                        length: V1AckVectorElement::MAX_LENGTH,
                    })
                    .collect(),
            };

            let encoded = encode_vec(&original).expect("encode");
            assert_eq!(encoded.len() % 4, 0, "structure must end on a DWORD boundary");
            assert_eq!(decode::<V1AckVectorHeader>(&encoded).expect("decode"), original);
        }
    }

    #[test]
    fn ack_vector_insufficient_bytes_for_count() {
        let bytes = [0x01]; // only 1 byte, need 2 for the count
        let result: DecodeResult<V1AckVectorHeader> = decode(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn ack_vector_insufficient_bytes_for_elements() {
        let bytes = [0x00, 0x05, 0x80]; // claims 5 elements, carries 1
        let result: DecodeResult<V1AckVectorHeader> = decode(&bytes);
        assert!(result.is_err());
    }

    // -- V1AckOfAcksHeader tests --

    #[test]
    fn ack_of_acks_roundtrip() {
        let original = V1AckOfAcksHeader {
            reset_seq_num: 0xDEAD_BEEF,
        };

        let encoded = encode_vec(&original).expect("encode");
        assert_eq!(
            encoded.as_slice(),
            &[0xDE, 0xAD, 0xBE, 0xEF],
            "[MS-RDPEUDP] 2.2 requires network byte order"
        );

        assert_eq!(decode::<V1AckOfAcksHeader>(&encoded).expect("decode"), original);
    }

    #[test]
    fn ack_of_acks_size() {
        let header = V1AckOfAcksHeader { reset_seq_num: 0 };
        assert_eq!(header.size(), 4);
    }

    // -- CorrelationIdPayload tests --

    #[test]
    fn correlation_id_roundtrip() {
        let original = CorrelationIdPayload {
            correlation_id: [
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
            ],
        };

        let encoded = encode_vec(&original).expect("encode");
        assert_eq!(encoded.len(), 32, "uCorrelationId(16) + uReserved(16)");
        assert_eq!(&encoded[16..], &[0u8; 16], "uReserved must be zeros");

        assert_eq!(decode::<CorrelationIdPayload>(&encoded).expect("decode"), original);
    }

    /// The correlation identifier from the section 4.1.1 SYN capture.
    #[test]
    fn correlation_id_from_the_spec_capture() {
        let mut bytes = [0u8; 32];
        let id = [
            0xD2, 0x35, 0xAC, 0x43, 0x89, 0x41, 0x42, 0xDA, 0xB1, 0x0E, 0xDD, 0x68, 0x87, 0xF7, 0xF9, 0xFB,
        ];
        bytes[..16].copy_from_slice(&id);

        let payload: CorrelationIdPayload = decode(&bytes).expect("decode");
        assert_eq!(payload.correlation_id, id);
        assert_eq!(encode_vec(&payload).expect("encode"), bytes);
    }

    #[test]
    fn correlation_id_insufficient_bytes() {
        let bytes = [0x01, 0x02, 0x03]; // only 3 bytes, need 32
        let result: DecodeResult<CorrelationIdPayload> = decode(&bytes);
        assert!(result.is_err());
    }
}
