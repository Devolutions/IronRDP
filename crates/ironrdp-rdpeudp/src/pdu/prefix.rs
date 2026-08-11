//! PacketPrefixByte and the wire-level byte-swap transform.
//!
//! MS-RDPEUDP2 Section 2.2.1.3 and Section 3.1.1.1.5.
//!
//! Every RDP-UDP2 packet on the wire is preceded by a PacketPrefixByte
//! that is byte-swapped with the 8th byte of the payload for obfuscation.
//! The send and receive transforms must be applied at the network boundary.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
/// Position of the PacketPrefixByte after the swap (0-indexed).
const PREFIX_SWAP_INDEX: usize = 7;

/// Minimum wire payload size. Packets shorter than this are padded.
const MIN_WIRE_SIZE: usize = 8; // 1 byte prefix + 7 bytes minimum

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// PacketPrefixByte structure.
///
/// MS-RDPEUDP2 Section 2.2.1.3.
/// Wire layout (1 byte, bits numbered least significant first):
/// - bit 0: Reserved (must be 0).
/// - bits 1-4: Packet_Type_Index (0 = normal, 8 = dummy).
/// - bits 5-7: Short_Packet_Length (size of following packet if < 7, else 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketPrefixByte {
    /// Packet type. Must be 0 (normal) or 8 (dummy).
    /// Dummy packets are treated normally by the transport layer
    /// but their loss does not trigger retransmit, and their
    /// contents are ignored by higher layers.
    pub packet_type_index: u8,

    /// Length of the following RDP-UDP2 packet if < 7 bytes.
    /// Set to 7 if the packet is >= 7 bytes.
    pub short_packet_length: u8,
}

impl PacketPrefixByte {
    /// Create a prefix byte for a normal packet.
    pub fn normal(packet_size: usize) -> Self {
        Self {
            packet_type_index: 0,
            short_packet_length: Self::compute_short_length(packet_size),
        }
    }

    /// Create a prefix byte for a dummy packet.
    pub fn dummy(packet_size: usize) -> Self {
        Self {
            packet_type_index: 8,
            short_packet_length: Self::compute_short_length(packet_size),
        }
    }

    /// Whether this is a dummy packet.
    pub fn is_dummy(self) -> bool {
        self.packet_type_index == 8
    }

    fn compute_short_length(packet_size: usize) -> u8 {
        if packet_size < 7 {
            // The guard ensures packet_size fits in u8.
            u8::try_from(packet_size).expect("packet_size < 7 fits in u8")
        } else {
            7
        }
    }

    /// Encode to a single byte.
    ///
    /// [MS-RDPEUDP2] 2.2.1.3 numbers the bits least-significant first, as its
    /// little-endian byte order implies: bit 0 is Reserved, bits 1 to 4 are
    /// Packet_Type_Index, bits 5 to 7 are Short_Packet_Length. The worked
    /// example in 3.1.1.1.5.1 pins it down: a prefix byte of 0x10 is
    /// Packet_Type_Index 8, which is the dummy-packet value, and is only
    /// reachable under this numbering.
    fn to_byte(self) -> u8 {
        ((self.short_packet_length & 0x07) << 5) | ((self.packet_type_index & 0x0F) << 1)
    }

    /// Decode from a single byte.
    fn from_byte(byte: u8) -> Self {
        Self {
            packet_type_index: (byte >> 1) & 0x0F,
            short_packet_length: (byte >> 5) & 0x07,
        }
    }
}

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Error returned by prefix encoding/decoding operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixError {
    /// Wire payload is too short for the swap operation.
    PayloadTooShort { len: usize },
    /// The packet_type_index is not 0 or 8.
    InvalidPacketType { got: u8 },
}

impl core::fmt::Display for PrefixError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PayloadTooShort { len } => {
                write!(f, "wire payload too short for prefix swap: {len} bytes")
            }
            Self::InvalidPacketType { got } => {
                write!(f, "invalid packet_type_index: {got}, expected 0 or 8")
            }
        }
    }
}

impl core::error::Error for PrefixError {}

/// Apply the send transform: prepend `PacketPrefixByte` and swap byte[0] with byte[7].
///
/// MS-RDPEUDP2 Section 3.1.1.1.5.1.
///
/// Takes the raw RDP-UDP2 packet bytes and writes the wire-format payload
/// (with prefix byte-swapped into position 7) into `wire_buf`.
///
/// Returns the number of bytes written to `wire_buf`.
///
/// # Procedure
///
/// 1. Generate PacketPrefixByte based on packet type and size.
/// 2. If the RDP-UDP2 packet is shorter than 7 bytes, pad to 7.
/// 3. Prepend the PacketPrefixByte (now at position 0).
/// 4. Swap byte[0] with byte[7].
pub fn encode_with_prefix(packet: &[u8], is_dummy: bool, wire_buf: &mut Vec<u8>) -> Result<usize, PrefixError> {
    let prefix = if is_dummy {
        PacketPrefixByte::dummy(packet.len())
    } else {
        PacketPrefixByte::normal(packet.len())
    };

    // Step 1-2: Determine padded size
    let padded_size = core::cmp::max(packet.len(), 7);

    // Step 3: Build [prefix_byte, packet_bytes..., padding...]
    wire_buf.clear();
    wire_buf.reserve(1 + padded_size);
    wire_buf.push(prefix.to_byte());
    wire_buf.extend_from_slice(packet);

    // Pad if necessary
    while wire_buf.len() < 1 + padded_size {
        wire_buf.push(0);
    }

    // Step 4: Swap byte[0] with byte[7]
    // After prepending prefix, wire_buf is [prefix, pkt[0], pkt[1], ..., pkt[6], ...]
    // wire_buf[0] = prefix_byte, wire_buf[7] = pkt[6] (or padding)
    if wire_buf.len() < MIN_WIRE_SIZE {
        return Err(PrefixError::PayloadTooShort { len: wire_buf.len() });
    }
    wire_buf.swap(0, PREFIX_SWAP_INDEX);

    Ok(wire_buf.len())
}

/// Apply the receive transform: swap byte[0] with byte[7], extract prefix, strip padding.
///
/// MS-RDPEUDP2 Section 3.1.1.1.5.2.
///
/// Takes the raw wire payload and returns `(prefix, packet_bytes)`.
///
/// # Procedure
///
/// 1. Swap byte[0] with byte[7].
/// 2. Remove byte[0]: this is the PacketPrefixByte.
/// 3. If Short_Packet_Length != 0 and < 7, truncate to that length.
/// 4. Remaining bytes are the RDP-UDP2 packet.
pub fn decode_with_prefix(wire: &mut [u8]) -> Result<(PacketPrefixByte, &[u8]), PrefixError> {
    if wire.len() < MIN_WIRE_SIZE {
        return Err(PrefixError::PayloadTooShort { len: wire.len() });
    }

    // Step 1: Swap byte[0] with byte[7]
    wire.swap(0, PREFIX_SWAP_INDEX);

    // Step 2: Extract the prefix byte (now at position 0)
    let prefix = PacketPrefixByte::from_byte(wire[0]);

    // Validate packet_type_index
    if prefix.packet_type_index != 0 && prefix.packet_type_index != 8 {
        return Err(PrefixError::InvalidPacketType {
            got: prefix.packet_type_index,
        });
    }

    // Step 3: The packet data starts at wire[1..]
    let packet_data = &wire[1..];

    // Step 4: If Short_Packet_Length is non-zero and less than 7,
    // the actual packet is only that many bytes (rest is padding).
    let actual_len = if prefix.short_packet_length > 0 && prefix.short_packet_length < 7 {
        usize::from(prefix.short_packet_length)
    } else {
        packet_data.len()
    };

    if actual_len > packet_data.len() {
        return Err(PrefixError::PayloadTooShort { len: packet_data.len() });
    }

    Ok((prefix, &packet_data[..actual_len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_byte_encode_decode() {
        let prefix = PacketPrefixByte {
            packet_type_index: 0,
            short_packet_length: 7,
        };
        let byte = prefix.to_byte();
        let decoded = PacketPrefixByte::from_byte(byte);
        assert_eq!(prefix, decoded);
    }

    #[test]
    fn prefix_byte_dummy_encode_decode() {
        let prefix = PacketPrefixByte {
            packet_type_index: 8,
            short_packet_length: 4,
        };
        let byte = prefix.to_byte();
        // Short_Packet_Length 4 in bits 5-7 and Packet_Type_Index 8 in bits 1-4:
        // 0b100_1000_0 = 0x90.
        assert_eq!(byte, 0x90);
        let decoded = PacketPrefixByte::from_byte(byte);
        assert_eq!(prefix, decoded);
    }

    /// MS-RDPEUDP2 Section 3.1.1.1.5.1 worked example.
    ///
    /// Input: 10 bytes [0x30, 0x35, 0x56, 0x78, 0xa2, 0x36, 0x73, 0xee, 0x68, 0xf2]
    /// PacketPrefixByte = 0x10, which is Packet_Type_Index 8 (a dummy packet) and
    /// Short_Packet_Length 0. This example is what fixes the bit numbering: 8 is
    /// a legal type and 2 is not, and only least-significant-first numbering
    /// reads 0x10 as 8.
    ///
    /// After prepend: [0x10, 0x30, 0x35, 0x56, 0x78, 0xa2, 0x36, 0x73, 0xee, 0x68, 0xf2]
    /// After swap[0]↔[7]: [0x73, 0x30, 0x35, 0x56, 0x78, 0xa2, 0x36, 0x10, 0xee, 0x68, 0xf2]
    #[test]
    fn spec_example_send_transform() {
        let packet = [0x30, 0x35, 0x56, 0x78, 0xa2, 0x36, 0x73, 0xee, 0x68, 0xf2];

        // Manually construct the prefix byte 0x10 for this example.
        // 0x10 = 0b0001_0000 → packet_type_index = (0x10 >> 1) & 0x0F = 8,
        //                      short_packet_length = (0x10 >> 5) & 0x07 = 0.
        // The spec uses this value in its example; we replicate it.
        let prefix_byte: u8 = 0x10;
        let prefix = PacketPrefixByte::from_byte(prefix_byte);
        assert_eq!(prefix.to_byte(), 0x10);

        // Build wire payload manually following the spec procedure
        let mut wire_buf = Vec::new();
        wire_buf.push(prefix_byte);
        wire_buf.extend_from_slice(&packet);
        // Length = 11, no padding needed (packet >= 7)

        // Swap byte[0] with byte[7]
        wire_buf.swap(0, 7);

        assert_eq!(
            wire_buf,
            vec![0x73, 0x30, 0x35, 0x56, 0x78, 0xa2, 0x36, 0x10, 0xee, 0x68, 0xf2]
        );
    }

    /// Verify the receive transform reverses the send transform.
    ///
    /// Exercises the swap mechanics directly, rather than through
    /// `decode_with_prefix`, so the assertion is about the prefix byte
    /// itself rather than the length/padding handling that function also
    /// does.
    #[test]
    fn spec_example_receive_transform() {
        let mut wire = vec![0x73, 0x30, 0x35, 0x56, 0x78, 0xa2, 0x36, 0x10, 0xee, 0x68, 0xf2];

        // Step 1: Swap byte[0] with byte[7]
        wire.swap(0, 7);

        // After swap: prefix byte is now at position 0
        let prefix = PacketPrefixByte::from_byte(wire[0]);
        assert_eq!(prefix.to_byte(), 0x10);
        assert_eq!(prefix.packet_type_index, 8, "0x10 is the dummy-packet type");
        assert_eq!(prefix.short_packet_length, 0);

        // Step 2: The original packet is everything after the prefix byte
        let packet = &wire[1..];
        assert_eq!(packet, &[0x30, 0x35, 0x56, 0x78, 0xa2, 0x36, 0x73, 0xee, 0x68, 0xf2]);
    }

    /// Only Packet_Type_Index 0 and 8 are legal, and the encoding must place
    /// them where a peer looks for them.
    ///
    /// [MS-RDPEUDP2] 3.1.1.1.5 fixes the two values, so any layout that cannot
    /// represent both without touching the reserved bit is wrong.
    #[test]
    fn prefix_byte_layout_matches_the_legal_type_values() {
        for length in 0..=7u8 {
            for packet_type_index in [0u8, 8] {
                let prefix = PacketPrefixByte {
                    packet_type_index,
                    short_packet_length: length,
                };
                let byte = prefix.to_byte();

                assert_eq!(byte & 0x01, 0, "the reserved bit must stay clear");
                assert_eq!(PacketPrefixByte::from_byte(byte), prefix);
            }
        }
    }
}
