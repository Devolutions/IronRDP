//! RDP-UDP2 Packet Header: the mandatory 2-byte header for all v2 packets.
//!
//! MS-RDPEUDP2 Section 2.2.1.1.
//! Wire layout: 16 bits = lower 12 bits Flags + upper 4 bits LogWindowSize.

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor};

use super::v2_flags::V2Flags;

/// Maximum value for LogWindowSize (4-bit field).
pub const LOG_WINDOW_SIZE_MAX: u8 = 15;

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Mandatory header for every RDP-UDP2 packet.
///
/// MS-RDPEUDP2 Section 2.2.1.1.
/// The 16-bit word is packed as: `flags(12 bits) | log_window_size(4 bits)`.
/// - `flags`: bitmap of `V2Flags` indicating optional payloads.
/// - `log_window_size`: log2 of the receiving window size (0..=15).
///   Actual window size = `1 << log_window_size`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V2Header {
    /// Which optional payloads are present.
    pub flags: V2Flags,

    /// log2 of the receiving window size announced by this endpoint.
    /// Must be in 0..=15.
    pub log_window_size: u8,
}

impl V2Header {
    const FIXED_PART_SIZE: usize = 2;
    const NAME: &'static str = "RDP-UDP2 Packet Header";

    /// Pack flags and LogWindowSize into a single 16-bit wire value.
    fn to_wire(self) -> u16 {
        let flags_bits = self.flags.bits() & V2Flags::MASK;
        let window_bits = u16::from(self.log_window_size & 0x0F) << 12;
        flags_bits | window_bits
    }

    /// Unpack from the 16-bit wire value.
    fn from_wire(wire: u16) -> DecodeResult<Self> {
        let flags = V2Flags::from_bits_truncate(wire & V2Flags::MASK);
        // The 4-bit field always fits in u8; the mask guarantees 0..=15.
        let log_window_size = u8::try_from((wire >> 12) & 0x0F).expect("4-bit field fits in u8");

        if !flags.ack_flags_valid() {
            return Err(ironrdp_core::invalid_field_err!(
                "RDP-UDP2 Packet Header",
                "flags",
                "ACK and ACKVEC are mutually exclusive"
            ));
        }

        Ok(Self { flags, log_window_size })
    }

    /// Actual window size (in packets): `1 << log_window_size`.
    pub fn window_size(self) -> u32 {
        1u32 << u32::from(self.log_window_size)
    }
}

impl Encode for V2Header {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_fixed_part_size!(in: dst);

        if self.log_window_size > LOG_WINDOW_SIZE_MAX {
            return Err(ironrdp_core::invalid_field_err!(
                Self::NAME,
                "log_window_size",
                "exceeds 4-bit maximum (15)"
            ));
        }

        if !self.flags.ack_flags_valid() {
            return Err(ironrdp_core::invalid_field_err!(
                Self::NAME,
                "flags",
                "ACK and ACKVEC are mutually exclusive"
            ));
        }

        dst.write_u16(self.to_wire());
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for V2Header {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_fixed_part_size!(in: src);
        let wire = src.read_u16();
        Self::from_wire(wire)
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_core::{decode, encode_vec};

    use super::*;

    #[test]
    fn encode_data_ack_header() {
        let header = V2Header {
            flags: V2Flags::ACK | V2Flags::DATA,
            log_window_size: 6, // window = 64
        };
        let encoded = encode_vec(&header).expect("encode");
        // flags = ACK(0x001) | DATA(0x004) = 0x005
        // log_window_size = 6 → shifted = 0x6000
        // wire = 0x6005
        assert_eq!(encoded.as_slice(), &[0x05, 0x60]);
    }

    #[test]
    fn decode_data_ack_header() {
        let bytes = [0x05, 0x60]; // 0x6005
        let decoded: V2Header = decode(&bytes).expect("decode");
        assert_eq!(decoded.flags, V2Flags::ACK | V2Flags::DATA);
        assert_eq!(decoded.log_window_size, 6);
    }

    #[test]
    fn roundtrip() {
        let original = V2Header {
            flags: V2Flags::ACKVEC | V2Flags::AOA | V2Flags::OVERHEADSIZE,
            log_window_size: 10,
        };
        let encoded = encode_vec(&original).expect("encode");
        let decoded: V2Header = decode(&encoded).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn window_size_calculation() {
        let header = V2Header {
            flags: V2Flags::empty(),
            log_window_size: 0,
        };
        assert_eq!(header.window_size(), 1);

        let header = V2Header {
            flags: V2Flags::empty(),
            log_window_size: 6,
        };
        assert_eq!(header.window_size(), 64);

        let header = V2Header {
            flags: V2Flags::empty(),
            log_window_size: LOG_WINDOW_SIZE_MAX,
        };
        assert_eq!(header.window_size(), 32768);
    }

    #[test]
    fn max_log_window_size() {
        let header = V2Header {
            flags: V2Flags::DATA,
            log_window_size: LOG_WINDOW_SIZE_MAX,
        };
        let encoded = encode_vec(&header).expect("encode");
        let decoded: V2Header = decode(&encoded).expect("decode");
        assert_eq!(decoded.log_window_size, LOG_WINDOW_SIZE_MAX);
    }

    #[test]
    fn reject_ack_and_ackvec_both_set() {
        // Manually construct wire value with both ACK and ACKVEC
        let flags_bits = V2Flags::ACK.bits() | V2Flags::ACKVEC.bits(); // 0x009
        let wire = flags_bits | (6u16 << 12);
        let bytes = wire.to_le_bytes();
        let result: DecodeResult<V2Header> = decode(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn encode_rejects_log_window_size_above_max() {
        let header = V2Header {
            flags: V2Flags::DATA,
            log_window_size: LOG_WINDOW_SIZE_MAX + 1,
        };
        assert!(encode_vec(&header).is_err());
    }

    #[test]
    fn encode_rejects_ack_and_ackvec_both_set() {
        let header = V2Header {
            flags: V2Flags::ACK | V2Flags::ACKVEC,
            log_window_size: 6,
        };
        assert!(encode_vec(&header).is_err());
    }

    #[test]
    fn size_is_correct() {
        let header = V2Header {
            flags: V2Flags::DATA,
            log_window_size: 8,
        };
        assert_eq!(header.size(), 2);
    }

    #[test]
    fn insufficient_bytes() {
        let bytes = [0x05]; // only 1 byte, need 2
        let result: DecodeResult<V2Header> = decode(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn flags_only_lower_12_bits() {
        // The six flags [MS-RDPEUDP2] 2.2.1.1 defines and no others:
        // 0x001, 0x004, 0x008, 0x010, 0x040, 0x100 = 0x15D.
        // from_bits_truncate drops undefined bit positions.
        let all_defined = V2Flags::all();
        assert_eq!(all_defined.bits(), 0x15D);

        let header = V2Header {
            flags: all_defined,
            log_window_size: 0xF,
        };
        let wire = header.to_wire();
        assert_eq!(wire, 0xF15D); // 0x15D | (0xF << 12)

        // Can't decode all_defined because ACK and ACKVEC are both set.
        // Test with valid flags to verify the bit separation.
        let header = V2Header {
            flags: V2Flags::DATA | V2Flags::ACKVEC | V2Flags::DELAYACKINFO,
            log_window_size: 12,
        };
        let encoded = encode_vec(&header).expect("encode");
        let decoded: V2Header = decode(&encoded).expect("decode");
        assert_eq!(decoded.flags, header.flags);
        assert_eq!(decoded.log_window_size, 12);
    }
}
