//! V2 header flags per MS-RDPEUDP2 Section 2.2.1.1.
//!
//! The `Flags` field in the RDP-UDP2 Packet Header is a 12-bit bitmap
//! stored in the lower 12 bits of the 16-bit header word.

use bitflags::bitflags;

bitflags! {
    /// Flags in the RDP-UDP2 Packet Header.
    ///
    /// MS-RDPEUDP2 Section 2.2.1.1.
    /// Stored in the lower 12 bits of the 16-bit header word.
    /// The upper 4 bits are `LogWindowSize`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    pub struct V2Flags: u16 {
        /// ACK payload (Section 2.2.1.2.1) is present.
        /// Mutually exclusive with ACKVEC.
        const ACK = 0x001;

        /// DataHeader + DataBody payloads (Section 2.2.1.2.5, 2.2.1.2.7) are present.
        const DATA = 0x004;

        /// Acknowledgement Vector payload (Section 2.2.1.2.6) is present.
        /// Mutually exclusive with ACK.
        const ACKVEC = 0x008;

        /// AckOfAcks payload (Section 2.2.1.2.4) is present.
        const AOA = 0x010;

        /// OverheadSize payload (Section 2.2.1.2.2) is present.
        const OVERHEADSIZE = 0x040;

        /// DelayAckInfo payload (Section 2.2.1.2.3) is present.
        const DELAYACKINFO = 0x100;
    }
}

// This is the whole table. [MS-RDPEUDP2] 2.2.1.1 defines six flags and no
// others, so a bit outside this set is one no peer will read and none of them
// carries a meaning beyond "this payload is present".
//
// Two absences are deliberate, because both were once defined here:
//
// - There is no DUMMY flag. A dummy packet is marked by Packet_Type_Index 8
//   in the PacketPrefixByte, one layer down (3.1.1.1.5, `crate::pdu::prefix`).
// - There are no CN or CWR flags. Those belong to MS-RDPEUDP, whose
//   RDPUDP_FEC_HEADER carries RDPUDP_FLAG_CN 0x0020 and RDPUDP_FLAG_CWR
//   0x0040 (see `crate::pdu::v1_flags`). MS-RDPEUDP2 has no explicit
//   congestion signalling at all: it names a Congestion Controller in
//   3.1.1.2.2 as a higher-layer concept and leaves it to infer conditions
//   from what it observes.

impl V2Flags {
    /// Mask for the 12-bit flags field within the 16-bit header word.
    pub const MASK: u16 = 0x0FFF;

    /// Validate that ACK and ACKVEC are not both set.
    /// Returns `true` if the flags are valid.
    pub fn ack_flags_valid(self) -> bool {
        !self.contains(Self::ACK | Self::ACKVEC)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_values_match_spec() {
        assert_eq!(V2Flags::ACK.bits(), 0x001);
        assert_eq!(V2Flags::DATA.bits(), 0x004);
        assert_eq!(V2Flags::ACKVEC.bits(), 0x008);
        assert_eq!(V2Flags::AOA.bits(), 0x010);
        assert_eq!(V2Flags::OVERHEADSIZE.bits(), 0x040);
        assert_eq!(V2Flags::DELAYACKINFO.bits(), 0x100);
    }

    #[test]
    fn all_flags_fit_in_12_bits() {
        let all = V2Flags::all();
        assert_eq!(all.bits() & !V2Flags::MASK, 0, "flags must fit in 12 bits");
    }

    #[test]
    fn ack_and_ackvec_mutual_exclusion() {
        let ack_only = V2Flags::ACK;
        assert!(ack_only.ack_flags_valid());

        let ackvec_only = V2Flags::ACKVEC;
        assert!(ackvec_only.ack_flags_valid());

        let both = V2Flags::ACK | V2Flags::ACKVEC;
        assert!(!both.ack_flags_valid());
    }

    #[test]
    fn data_packet_flags() {
        let flags = V2Flags::ACK | V2Flags::DATA;
        assert!(flags.contains(V2Flags::ACK));
        assert!(flags.contains(V2Flags::DATA));
        assert!(flags.ack_flags_valid());
    }

    #[test]
    fn roundtrip_from_bits() {
        let original = V2Flags::DATA | V2Flags::ACKVEC | V2Flags::DELAYACKINFO;
        let bits = original.bits();
        let restored = V2Flags::from_bits_truncate(bits);
        assert_eq!(original, restored);
    }
}
