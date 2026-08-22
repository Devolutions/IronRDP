//! V1 header flags per MS-RDPEUDP Section 2.2.2.1.
//!
//! The `uFlags` field in `RDPUDP_FEC_HEADER` is a 16-bit bitmap
//! indicating which optional payloads are present and which
//! protocol features are active.

use bitflags::bitflags;

bitflags! {
    /// Flags in the RDPUDP_FEC_HEADER `uFlags` field.
    ///
    /// MS-RDPEUDP Section 2.2.2.1.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    pub struct V1Flags: u16 {
        /// SYN: connection initialization.
        const SYN = 0x0001;

        /// FIN: connection teardown (currently unused by the spec).
        const FIN = 0x0002;

        /// ACK: `RDPUDP_ACK_VECTOR_HEADER` is present, except on a SYN+ACK
        /// (section 3.1.5.1.3), where it marks snSourceAck as meaningful and
        /// no vector follows.
        const ACK = 0x0004;

        /// DATA: `RDPUDP_SOURCE_PAYLOAD_HEADER` or
        /// `RDPUDP_FEC_PAYLOAD_HEADER` follows.
        const DATA = 0x0008;

        /// FEC: `RDPUDP_FEC_PAYLOAD_HEADER` is present.
        const FEC = 0x0010;

        /// Congestion Notification: receiver detected packet loss.
        const CN = 0x0020;

        /// Congestion Window Reset: sender reacted to CN.
        const CWR = 0x0040;

        /// SACK option (not used).
        const SACK_OPTION = 0x0080;

        /// ACK-of-ACKs: `RDPUDP_ACK_OF_ACKVECTOR_HEADER` is present.
        const ACK_OF_ACKS = 0x0100;

        /// Connection does not require persistent retransmits (lossy mode).
        const SYNLOSSY = 0x0200;

        /// Receiver delayed generating this ACK; do not use for RTT estimation.
        const ACKDELAYED = 0x0400;

        /// `RDPUDP_CORRELATION_ID_PAYLOAD` is present.
        const CORRELATION_ID = 0x0800;

        /// `RDPUDP_SYNDATAEX_PAYLOAD` is present.
        const SYNEX = 0x1000;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_values_match_spec() {
        assert_eq!(V1Flags::SYN.bits(), 0x0001);
        assert_eq!(V1Flags::FIN.bits(), 0x0002);
        assert_eq!(V1Flags::ACK.bits(), 0x0004);
        assert_eq!(V1Flags::DATA.bits(), 0x0008);
        assert_eq!(V1Flags::FEC.bits(), 0x0010);
        assert_eq!(V1Flags::CN.bits(), 0x0020);
        assert_eq!(V1Flags::CWR.bits(), 0x0040);
        assert_eq!(V1Flags::SACK_OPTION.bits(), 0x0080);
        assert_eq!(V1Flags::ACK_OF_ACKS.bits(), 0x0100);
        assert_eq!(V1Flags::SYNLOSSY.bits(), 0x0200);
        assert_eq!(V1Flags::ACKDELAYED.bits(), 0x0400);
        assert_eq!(V1Flags::CORRELATION_ID.bits(), 0x0800);
        assert_eq!(V1Flags::SYNEX.bits(), 0x1000);
    }

    #[test]
    fn syn_datagram_flags() {
        // A typical SYN datagram sets SYN + SYNEX
        let flags = V1Flags::SYN | V1Flags::SYNEX;
        assert!(flags.contains(V1Flags::SYN));
        assert!(flags.contains(V1Flags::SYNEX));
        assert!(!flags.contains(V1Flags::ACK));
    }

    #[test]
    fn syn_ack_datagram_flags() {
        // SYN+ACK sets SYN + ACK + SYNEX
        let flags = V1Flags::SYN | V1Flags::ACK | V1Flags::SYNEX;
        assert!(flags.contains(V1Flags::SYN));
        assert!(flags.contains(V1Flags::ACK));
        assert!(flags.contains(V1Flags::SYNEX));
    }

    #[test]
    fn roundtrip_from_bits() {
        let original = V1Flags::SYN | V1Flags::CN | V1Flags::CORRELATION_ID;
        let bits = original.bits();
        let restored = V1Flags::from_bits_truncate(bits);
        assert_eq!(original, restored);
    }
}
