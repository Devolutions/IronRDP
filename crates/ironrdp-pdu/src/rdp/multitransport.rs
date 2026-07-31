//! Initiate Multitransport Request and Response PDU types.
//!
//! Defined in [\[MS-RDPBCGR\] 2.2.15.1] and [\[MS-RDPBCGR\] 2.2.15.2].
//!
//! [\[MS-RDPBCGR\] 2.2.15.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/de783158-8b01-4818-8fb0-62523a5b3490
//! [\[MS-RDPBCGR\] 2.2.15.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/44044233-e498-46f8-8e16-1ffa595a8e8b

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, invalid_field_err,
    read_padding, write_padding,
};

use crate::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};

/// Length of the security cookie used for transport binding validation.
const SECURITY_COOKIE_LEN: usize = 16;

/// Requested transport protocol for multitransport bootstrapping.
///
/// Defined in [\[MS-RDPBCGR\] 2.2.15.1], `requestedProtocol` field.
///
/// [\[MS-RDPBCGR\] 2.2.15.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/de783158-8b01-4818-8fb0-62523a5b3490
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[repr(u16)]
pub enum RequestedProtocol {
    /// Reliable UDP transport (RDPEUDP2 + TLS).
    ///
    /// `INITIATE_REQUEST_PROTOCOL_UDPFECR`
    UdpFecR = 0x0001,
    /// Lossy UDP transport (RDPEUDP + DTLS, with forward error correction).
    ///
    /// `INITIATE_REQUEST_PROTOCOL_UDPFECL`
    UdpFecL = 0x0002,
}

impl RequestedProtocol {
    fn from_u16(val: u16) -> Option<Self> {
        match val {
            0x0001 => Some(Self::UdpFecR),
            0x0002 => Some(Self::UdpFecL),
            _ => None,
        }
    }

    #[expect(
        clippy::as_conversions,
        reason = "repr(u16) guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn as_u16(self) -> u16 {
        self as u16
    }
}

/// Server Initiate Multitransport Request PDU.
///
/// Sent by the server on the IO channel after licensing to bootstrap a
/// sideband UDP transport. The `request_id` and `security_cookie` are
/// echoed by the client in the tunnel creation request over the new
/// transport, binding the two connections together.
///
/// A server may send up to two of these — one for reliable and one for
/// lossy UDP.
///
/// Defined in [\[MS-RDPBCGR\] 2.2.15.1].
///
/// [\[MS-RDPBCGR\] 2.2.15.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/de783158-8b01-4818-8fb0-62523a5b3490
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MultitransportRequestPdu {
    pub security_header: BasicSecurityHeader,
    /// Unique ID correlating this request with the tunnel creation request.
    pub request_id: u32,
    /// Which transport protocol the server is requesting.
    pub requested_protocol: RequestedProtocol,
    /// 16-byte random cookie for transport binding validation.
    pub security_cookie: [u8; SECURITY_COOKIE_LEN],
}

impl MultitransportRequestPdu {
    const NAME: &'static str = "MultitransportRequestPdu";

    const FIXED_PART_SIZE: usize = BasicSecurityHeader::FIXED_PART_SIZE
        + 4 /* requestId */
        + 2 /* requestedProtocol */
        + 2 /* reserved */
        + SECURITY_COOKIE_LEN /* securityCookie */;
}

impl Encode for MultitransportRequestPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.security_header.encode(dst)?;
        dst.write_u32(self.request_id);
        dst.write_u16(self.requested_protocol.as_u16());
        write_padding!(dst, 2);
        dst.write_slice(&self.security_cookie);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for MultitransportRequestPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let security_header = BasicSecurityHeader::decode(src)?;

        // MS-RDPBCGR 2.2.15.1 requires the flags to *contain* SEC_TRANSPORT_REQ,
        // and 2.2.8.1.1.2.1 says SEC_RESET_SEQNO and SEC_IGNORE_SEQNO "MUST be
        // ignored", so they are masked off before comparing rather than treated
        // as a mismatch.
        //
        // What remains is compared for equality, which is stricter than the
        // spec requires and deliberately so: a successful decode is what tells
        // the connector that what arrived really is a request. The connector
        // narrows by MCS channel first, but the message channel also carries
        // auto-detect traffic, whose SEC_AUTODETECT_REQ/RSP a bare `contains`
        // check would happily accept as a multitransport request.
        let flags = security_header
            .flags
            .difference(BasicSecurityHeaderFlags::RESET_SEQNO | BasicSecurityHeaderFlags::IGNORE_SEQNO);
        if flags != BasicSecurityHeaderFlags::TRANSPORT_REQ {
            return Err(invalid_field_err!(
                "securityHeader",
                "expected securityHeader flags to contain SEC_TRANSPORT_REQ and no other PDU-type flag"
            ));
        }

        let request_id = src.read_u32();

        let protocol_raw = src.read_u16();
        let requested_protocol = RequestedProtocol::from_u16(protocol_raw)
            .ok_or_else(|| invalid_field_err!("requestedProtocol", "unknown protocol value"))?;

        read_padding!(src, 2);

        let security_cookie: [u8; SECURITY_COOKIE_LEN] = src.read_array();

        Ok(Self {
            security_header,
            request_id,
            requested_protocol,
            security_cookie,
        })
    }
}

/// Client Initiate Multitransport Response PDU.
///
/// Sent by the client on the IO channel after the UDP transport is
/// established (or has failed). The `request_id` must match the
/// corresponding server request.
///
/// Defined in [\[MS-RDPBCGR\] 2.2.15.2].
///
/// [\[MS-RDPBCGR\] 2.2.15.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/44044233-e498-46f8-8e16-1ffa595a8e8b
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MultitransportResponsePdu {
    pub security_header: BasicSecurityHeader,
    /// Request ID matching the server's Initiate Multitransport Request.
    pub request_id: u32,
    /// HRESULT indicating success or failure of the transport setup.
    pub hr_response: u32,
}

impl MultitransportResponsePdu {
    const NAME: &'static str = "MultitransportResponsePdu";

    const FIXED_PART_SIZE: usize = BasicSecurityHeader::FIXED_PART_SIZE
        + 4 /* requestId */
        + 4 /* hrResponse */;

    /// `S_OK` — multitransport connection established.
    ///
    /// Per [\[MS-RDPBCGR\] 2.2.15.2], this MUST only be sent to servers that
    /// advertised `SOFTSYNC_TCP_TO_UDP` in the GCC `MultiTransportChannelData`.
    ///
    /// [\[MS-RDPBCGR\] 2.2.15.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/44044233-e498-46f8-8e16-1ffa595a8e8b
    pub const S_OK: u32 = 0x0000_0000;

    /// `E_ABORT` — client was unable to establish the multitransport connection.
    pub const E_ABORT: u32 = 0x8000_4004;

    /// Create a success response for the given request ID.
    pub fn success(request_id: u32) -> Self {
        Self {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_RSP,
            },
            request_id,
            hr_response: Self::S_OK,
        }
    }

    /// Create a failure response for the given request ID.
    pub fn abort(request_id: u32) -> Self {
        Self {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_RSP,
            },
            request_id,
            hr_response: Self::E_ABORT,
        }
    }

    /// Whether this response indicates success.
    pub fn is_success(&self) -> bool {
        self.hr_response == Self::S_OK
    }
}

impl Encode for MultitransportResponsePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.security_header.encode(dst)?;
        dst.write_u32(self.request_id);
        dst.write_u32(self.hr_response);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for MultitransportResponsePdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let security_header = BasicSecurityHeader::decode(src)?;

        // MS-RDPBCGR 2.2.15.2 requires the flags to *contain* SEC_TRANSPORT_RSP,
        // and the ignorable sequence-number flags are masked off first, exactly as
        // on the request side; see the note on `MultitransportRequestPdu::decode`
        // for why what remains is then compared for equality.
        let flags = security_header
            .flags
            .difference(BasicSecurityHeaderFlags::RESET_SEQNO | BasicSecurityHeaderFlags::IGNORE_SEQNO);
        if flags != BasicSecurityHeaderFlags::TRANSPORT_RSP {
            return Err(invalid_field_err!(
                "securityHeader",
                "expected securityHeader flags to contain SEC_TRANSPORT_RSP and no other PDU-type flag"
            ));
        }

        let request_id = src.read_u32();
        let hr_response = src.read_u32();

        Ok(Self {
            security_header,
            request_id,
            hr_response,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST_WIRE: &[u8] = &[
        // BasicSecurityHeader (4 bytes)
        0x02, 0x00, // flags = TRANSPORT_REQ (0x0002)
        0x00, 0x00, // flagsHi = 0
        // Payload (24 bytes)
        0x2A, 0x00, 0x00, 0x00, // requestId = 42
        0x01, 0x00, // requestedProtocol = UdpFecR (0x0001)
        0x00, 0x00, // reserved
        0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, // securityCookie
        0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB,
    ];

    const RESPONSE_SUCCESS_WIRE: &[u8] = &[
        // BasicSecurityHeader (4 bytes)
        0x04, 0x00, // flags = TRANSPORT_RSP (0x0004)
        0x00, 0x00, // flagsHi = 0
        // Payload (8 bytes)
        0x2A, 0x00, 0x00, 0x00, // requestId = 42
        0x00, 0x00, 0x00, 0x00, // hrResponse = S_OK
    ];

    const RESPONSE_ABORT_WIRE: &[u8] = &[
        // BasicSecurityHeader (4 bytes)
        0x04, 0x00, // flags = TRANSPORT_RSP (0x0004)
        0x00, 0x00, // flagsHi = 0
        // Payload (8 bytes)
        0x07, 0x00, 0x00, 0x00, // requestId = 7
        0x04, 0x40, 0x00, 0x80, // hrResponse = E_ABORT (0x80004004)
    ];

    #[test]
    fn decode_request() {
        let pdu = ironrdp_core::decode::<MultitransportRequestPdu>(REQUEST_WIRE).unwrap();
        assert_eq!(pdu.request_id, 42);
        assert_eq!(pdu.requested_protocol, RequestedProtocol::UdpFecR);
        assert_eq!(pdu.security_cookie, [0xAB; 16]);
    }

    #[test]
    fn encode_request() {
        let pdu = MultitransportRequestPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
            },
            request_id: 42,
            requested_protocol: RequestedProtocol::UdpFecR,
            security_cookie: [0xAB; 16],
        };
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), REQUEST_WIRE);
    }

    #[test]
    fn request_round_trip() {
        let original = MultitransportRequestPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
            },
            request_id: 0xDEAD_BEEF,
            requested_protocol: RequestedProtocol::UdpFecL,
            security_cookie: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        };
        let encoded = ironrdp_core::encode_vec(&original).unwrap();
        let decoded = ironrdp_core::decode::<MultitransportRequestPdu>(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn request_size() {
        assert_eq!(MultitransportRequestPdu::FIXED_PART_SIZE, 28);
    }

    #[test]
    fn decode_response_success() {
        let pdu = ironrdp_core::decode::<MultitransportResponsePdu>(RESPONSE_SUCCESS_WIRE).unwrap();
        assert_eq!(pdu.request_id, 42);
        assert!(pdu.is_success());
    }

    #[test]
    fn decode_response_abort() {
        let pdu = ironrdp_core::decode::<MultitransportResponsePdu>(RESPONSE_ABORT_WIRE).unwrap();
        assert_eq!(pdu.request_id, 7);
        assert_eq!(pdu.hr_response, MultitransportResponsePdu::E_ABORT);
        assert!(!pdu.is_success());
    }

    #[test]
    fn encode_response_success() {
        let pdu = MultitransportResponsePdu::success(42);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), RESPONSE_SUCCESS_WIRE);
    }

    #[test]
    fn encode_response_abort() {
        let pdu = MultitransportResponsePdu::abort(7);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), RESPONSE_ABORT_WIRE);
    }

    #[test]
    fn response_round_trip() {
        let original = MultitransportResponsePdu::success(0xCAFE_BABE);
        let encoded = ironrdp_core::encode_vec(&original).unwrap();
        let decoded = ironrdp_core::decode::<MultitransportResponsePdu>(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn response_size() {
        assert_eq!(MultitransportResponsePdu::FIXED_PART_SIZE, 12);
    }

    #[test]
    fn decode_request_wrong_flags() {
        let bad_wire: &[u8] = &[
            0x04, 0x00, // flags = TRANSPORT_RSP (wrong for request)
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(ironrdp_core::decode::<MultitransportRequestPdu>(bad_wire).is_err());
    }

    /// MS-RDPBCGR 2.2.8.1.1.2.1 says SEC_RESET_SEQNO and SEC_IGNORE_SEQNO "MUST
    /// be ignored", and the spec's own worked examples carry them alongside other
    /// flags. Rejecting a PDU that sets them would abort the connection over bits
    /// the peer is told to disregard.
    ///
    /// Both directions are covered because the two decoders are symmetric: 2.2.15.1
    /// and 2.2.15.2 each say the flags MUST *contain* their respective discriminator.
    #[test]
    fn decode_ignores_the_seqno_flags_in_both_directions() {
        const RESET_SEQNO: u16 = 0x0010;
        const IGNORE_SEQNO: u16 = 0x0020;

        for (label, base) in [("request", 0x0002u16), ("response", 0x0004)] {
            for (variant, extra) in [
                ("SEC_RESET_SEQNO", RESET_SEQNO),
                ("SEC_IGNORE_SEQNO", IGNORE_SEQNO),
                ("both seqno flags", RESET_SEQNO | IGNORE_SEQNO),
            ] {
                let flags = base | extra;

                if base == 0x0002 {
                    let mut wire = REQUEST_WIRE.to_vec();
                    wire[0..2].copy_from_slice(&flags.to_le_bytes());
                    let pdu = ironrdp_core::decode::<MultitransportRequestPdu>(&wire)
                        .unwrap_or_else(|e| panic!("{label} with {variant} must decode: {e}"));
                    assert_eq!(pdu.request_id, 42, "{label} with {variant}");
                } else {
                    let mut wire = RESPONSE_SUCCESS_WIRE.to_vec();
                    wire[0..2].copy_from_slice(&flags.to_le_bytes());
                    let pdu = ironrdp_core::decode::<MultitransportResponsePdu>(&wire)
                        .unwrap_or_else(|e| panic!("{label} with {variant} must decode: {e}"));
                    assert_eq!(pdu.request_id, 42, "{label} with {variant}");
                }
            }
        }
    }

    /// The equality check past the ignorable flags is what keeps other traffic on
    /// the shared message channel from decoding as multitransport. Auto-detect is
    /// the case that actually shares the channel.
    #[test]
    fn decode_rejects_another_pdu_type_alongside_the_discriminator_in_both_directions() {
        const AUTODETECT_REQ: u16 = 0x1000;

        let mut request = REQUEST_WIRE.to_vec();
        request[0..2].copy_from_slice(&(0x0002u16 | AUTODETECT_REQ).to_le_bytes());
        assert!(ironrdp_core::decode::<MultitransportRequestPdu>(&request).is_err());

        let mut response = RESPONSE_SUCCESS_WIRE.to_vec();
        response[0..2].copy_from_slice(&(0x0004u16 | AUTODETECT_REQ).to_le_bytes());
        assert!(ironrdp_core::decode::<MultitransportResponsePdu>(&response).is_err());
    }

    #[test]
    fn decode_request_unknown_protocol() {
        let bad_wire: &[u8] = &[
            0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xFF, 0x00, // unknown protocol
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(ironrdp_core::decode::<MultitransportRequestPdu>(bad_wire).is_err());
    }
}
