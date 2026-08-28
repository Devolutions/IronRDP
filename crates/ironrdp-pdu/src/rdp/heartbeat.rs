//! Server Heartbeat PDU.
//!
//! Defined in [\[MS-RDPBCGR\] 2.2.16.1].
//!
//! [\[MS-RDPBCGR\] 2.2.16.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/86db1222-6b5c-4001-9923-6b292e1ccb47

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, invalid_field_err,
};

use crate::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};

/// Server Heartbeat PDU.
///
/// Sent by the server on the MCS message channel to let the client monitor
/// connection health in real time. Only sent when no other PDU has gone out
/// in the current heartbeat interval; the client is free to ignore `count1`
/// and `count2`.
///
/// Defined in [\[MS-RDPBCGR\] 2.2.16.1].
///
/// [\[MS-RDPBCGR\] 2.2.16.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/86db1222-6b5c-4001-9923-6b292e1ccb47
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct HeartbeatPdu {
    pub security_header: BasicSecurityHeader,
    /// Seconds between Heartbeat PDUs.
    pub period: u8,
    /// Missed heartbeats that SHOULD trigger a client-side warning. The client MAY ignore this.
    pub count1: u8,
    /// Missed heartbeats after the warning that SHOULD trigger a client-side reconnect attempt.
    /// The client MAY ignore this.
    pub count2: u8,
}

impl HeartbeatPdu {
    const NAME: &'static str = "HeartbeatPdu";

    const FIXED_PART_SIZE: usize = BasicSecurityHeader::FIXED_PART_SIZE
        + 1 /* reserved */
        + 1 /* period */
        + 1 /* count1 */
        + 1 /* count2 */;
}

impl Encode for HeartbeatPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.security_header.encode(dst)?;
        dst.write_u8(0); // reserved, MUST be zero
        dst.write_u8(self.period);
        dst.write_u8(self.count1);
        dst.write_u8(self.count2);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for HeartbeatPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let security_header = BasicSecurityHeader::decode(src)?;

        // MS-RDPBCGR 2.2.8.1.1.2.1 says SEC_RESET_SEQNO and SEC_IGNORE_SEQNO "MUST be
        // ignored", so they are masked off before comparing, matching
        // MultitransportRequestPdu/MultitransportResponsePdu's decode. What remains is
        // compared for equality rather than `contains`, so a successful decode is a
        // reliable signal that this really is a Heartbeat PDU and not some other
        // message-channel traffic (auto-detect) that happens to also set SEC_HEARTBEAT
        // alongside its own discriminator.
        let flags = security_header
            .flags
            .difference(BasicSecurityHeaderFlags::RESET_SEQNO | BasicSecurityHeaderFlags::IGNORE_SEQNO);
        if flags != BasicSecurityHeaderFlags::HEARTBEAT {
            return Err(invalid_field_err!(
                "securityHeader",
                "expected securityHeader flags to contain SEC_HEARTBEAT and no other PDU-type flag",
                in: src
            ));
        }

        let _reserved = src.read_u8();
        let period = src.read_u8();
        let count1 = src.read_u8();
        let count2 = src.read_u8();

        Ok(Self {
            security_header,
            period,
            count1,
            count2,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEARTBEAT_WIRE: &[u8] = &[
        // BasicSecurityHeader (4 bytes)
        0x00, 0x40, // flags = HEARTBEAT (0x4000)
        0x00, 0x00, // flagsHi = 0
        // Payload (4 bytes)
        0x00, // reserved
        0x05, // period = 5 seconds
        0x02, // count1 = 2
        0x03, // count2 = 3
    ];

    #[test]
    fn decode() {
        let pdu = ironrdp_core::decode::<HeartbeatPdu>(HEARTBEAT_WIRE).unwrap();
        assert_eq!(pdu.period, 5);
        assert_eq!(pdu.count1, 2);
        assert_eq!(pdu.count2, 3);
    }

    #[test]
    fn encode() {
        let pdu = HeartbeatPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::HEARTBEAT,
            },
            period: 5,
            count1: 2,
            count2: 3,
        };
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), HEARTBEAT_WIRE);
    }

    #[test]
    fn round_trip() {
        let original = HeartbeatPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::HEARTBEAT,
            },
            period: 30,
            count1: 4,
            count2: 8,
        };
        let encoded = ironrdp_core::encode_vec(&original).unwrap();
        let decoded = ironrdp_core::decode::<HeartbeatPdu>(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn size() {
        assert_eq!(HeartbeatPdu::FIXED_PART_SIZE, 8);
    }

    /// Same rationale as MultitransportRequestPdu's identical test: the spec says
    /// these two flags MUST be ignored, and rejecting a PDU that sets them would
    /// abort the connection over bits the peer is told to disregard.
    #[test]
    fn decode_ignores_the_seqno_flags() {
        const RESET_SEQNO: u16 = 0x0010;
        const IGNORE_SEQNO: u16 = 0x0020;

        for (variant, extra) in [
            ("SEC_RESET_SEQNO", RESET_SEQNO),
            ("SEC_IGNORE_SEQNO", IGNORE_SEQNO),
            ("both seqno flags", RESET_SEQNO | IGNORE_SEQNO),
        ] {
            let flags: u16 = 0x4000 | extra;
            let mut wire = HEARTBEAT_WIRE.to_vec();
            wire[0..2].copy_from_slice(&flags.to_le_bytes());
            let pdu =
                ironrdp_core::decode::<HeartbeatPdu>(&wire).unwrap_or_else(|e| panic!("{variant} must decode: {e}"));
            assert_eq!(pdu.period, 5, "{variant}");
        }
    }

    /// Mirrors MultitransportRequestPdu's cross-PDU-type rejection test: the
    /// message channel also carries auto-detect traffic, so a bare `contains`
    /// check on SEC_HEARTBEAT would happily accept an auto-detect PDU that
    /// happened to also set it. The masked-equality check must reject that.
    #[test]
    fn decode_rejects_another_pdu_type_alongside_the_discriminator() {
        const AUTODETECT_REQ: u16 = 0x1000;

        let mut wire = HEARTBEAT_WIRE.to_vec();
        wire[0..2].copy_from_slice(&(0x4000u16 | AUTODETECT_REQ).to_le_bytes());
        assert!(ironrdp_core::decode::<HeartbeatPdu>(&wire).is_err());
    }
}
