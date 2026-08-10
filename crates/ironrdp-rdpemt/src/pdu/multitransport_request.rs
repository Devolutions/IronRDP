//! Server Initiate Multitransport Request PDU payload.
//!
//! MS-RDPBCGR Section 2.2.15.1.
//!
//! This is the **inner payload** after the TPKT + X224 + MCS + security
//! header have been stripped by the TCP-side PDU stack. The server sends
//! this on the MCS message channel after licensing to bootstrap a
//! sideband UDP transport.
//!
//! Wire layout (24 bytes):
//! ```text
//! Bytes 0-3:   RequestID (u32 LE)
//! Bytes 4-5:   RequestedProtocol (u16 LE)
//! Bytes 6-7:   Reserved (u16 LE, MUST be 0)
//! Bytes 8-23:  SecurityCookie (16 bytes)
//! ```

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, InvalidFieldErr as _, ReadCursor, WriteCursor};

use crate::TunnelConfig;
use crate::pdu::create_request::SECURITY_COOKIE_LEN;

/// Total payload size: RequestID(4) + RequestedProtocol(2) + Reserved(2) + Cookie(16).
const WIRE_SIZE: usize = 4 + 2 + 2 + SECURITY_COOKIE_LEN;

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Requested transport protocol for multitransport bootstrapping.
///
/// MS-RDPBCGR §2.2.15.1: `requestedProtocol` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum RequestedProtocol {
    /// Reliable UDP (RDPEUDP2 + TLS).
    /// `INITIATE_REQUEST_PROTOCOL_UDPFECR`
    ReliableUdp = 0x0001,
    /// Lossy UDP (RDPEUDP1 + DTLS, with FEC).
    /// `INITIATE_REQUEST_PROTOCOL_UDPFECL`
    LossyUdp = 0x0002,
}

impl RequestedProtocol {
    fn from_u16(val: u16) -> Option<Self> {
        match val {
            0x0001 => Some(Self::ReliableUdp),
            0x0002 => Some(Self::LossyUdp),
            _ => None,
        }
    }

    #[expect(clippy::as_conversions, reason = "repr(u16) → u16 is lossless")]
    fn to_u16(self) -> u16 {
        self as u16
    }
}

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Server Initiate Multitransport Request PDU payload.
///
/// Sent by the server on the MCS message channel after licensing.
/// The client uses `request_id` and `security_cookie` to construct
/// the RDPEMT Tunnel Create Request over the new UDP connection.
///
/// A maximum of two of these PDUs can be sent per connection: one
/// for reliable UDP and one for lossy UDP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultitransportRequest {
    /// Unique ID correlating this request with the Tunnel Create Request
    /// that the client will send over the new transport.
    pub request_id: u32,
    /// Which transport protocol the server is requesting.
    pub requested_protocol: RequestedProtocol,
    /// 16-byte random security cookie for transport binding validation.
    pub security_cookie: [u8; SECURITY_COOKIE_LEN],
}

impl MultitransportRequest {
    const NAME: &'static str = "MultitransportRequest";

    /// Convert to a [`TunnelConfig`] for use with `connect_udp()`.
    ///
    /// Extracts the `request_id` and `security_cookie` that the RDPEMT
    /// Tunnel Create Request needs.
    pub fn to_tunnel_config(&self) -> TunnelConfig {
        TunnelConfig {
            request_id: self.request_id,
            security_cookie: self.security_cookie,
        }
    }
}

impl Encode for MultitransportRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.request_id);
        dst.write_u16(self.requested_protocol.to_u16());
        dst.write_u16(0); // Reserved
        dst.write_slice(&self.security_cookie);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        WIRE_SIZE
    }
}

impl Decode<'_> for MultitransportRequest {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_core::ensure_size!(in: src, size: WIRE_SIZE);

        let request_id = src.read_u32();

        let protocol_raw = src.read_u16();
        let requested_protocol = RequestedProtocol::from_u16(protocol_raw).ok_or_else(|| {
            ironrdp_core::DecodeError::invalid_field(Self::NAME, "requestedProtocol", "unknown protocol value")
        })?;

        let _reserved = src.read_u16();

        let security_cookie: [u8; SECURITY_COOKIE_LEN] = src.read_array();

        Ok(Self {
            request_id,
            requested_protocol,
            security_cookie,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manually constructed wire bytes matching the spec layout.
    const WIRE_EXAMPLE: &[u8] = &[
        0x2A, 0x00, 0x00, 0x00, // RequestID = 42
        0x01, 0x00, // RequestedProtocol = ReliableUdp (0x0001)
        0x00, 0x00, // Reserved = 0
        0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, // SecurityCookie (16 bytes)
        0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB,
    ];

    #[test]
    fn decode_wire_example() {
        let pdu: MultitransportRequest = ironrdp_core::decode(WIRE_EXAMPLE).expect("decode");
        assert_eq!(pdu.request_id, 42);
        assert_eq!(pdu.requested_protocol, RequestedProtocol::ReliableUdp);
        assert_eq!(pdu.security_cookie, [0xAB; 16]);
    }

    #[test]
    fn encode_matches_wire_example() {
        let pdu = MultitransportRequest {
            request_id: 42,
            requested_protocol: RequestedProtocol::ReliableUdp,
            security_cookie: [0xAB; 16],
        };

        let encoded = ironrdp_core::encode_vec(&pdu).expect("encode");
        assert_eq!(encoded.as_slice(), WIRE_EXAMPLE);
    }

    #[test]
    fn round_trip() {
        let original = MultitransportRequest {
            request_id: 0xDEAD_BEEF,
            requested_protocol: RequestedProtocol::LossyUdp,
            security_cookie: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        };

        let encoded = ironrdp_core::encode_vec(&original).expect("encode");
        let decoded: MultitransportRequest = ironrdp_core::decode(&encoded).expect("decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn wire_size_is_24() {
        let pdu = MultitransportRequest {
            request_id: 0,
            requested_protocol: RequestedProtocol::ReliableUdp,
            security_cookie: [0; 16],
        };
        assert_eq!(pdu.size(), 24);
    }

    #[test]
    fn to_tunnel_config_extracts_fields() {
        let pdu = MultitransportRequest {
            request_id: 7,
            requested_protocol: RequestedProtocol::ReliableUdp,
            security_cookie: [0xCC; 16],
        };

        let config = pdu.to_tunnel_config();
        assert_eq!(config.request_id, 7);
        assert_eq!(config.security_cookie, [0xCC; 16]);
    }

    #[test]
    fn requested_protocol_discriminants() {
        assert_eq!(RequestedProtocol::ReliableUdp.to_u16(), 0x0001);
        assert_eq!(RequestedProtocol::LossyUdp.to_u16(), 0x0002);
    }

    #[test]
    fn decode_rejects_unknown_protocol() {
        let wire: &[u8] = &[
            0x01, 0x00, 0x00, 0x00, // RequestID = 1
            0xFF, 0x00, // RequestedProtocol = 0x00FF (invalid)
            0x00, 0x00, // Reserved
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Cookie
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        let result: DecodeResult<MultitransportRequest> = ironrdp_core::decode(wire);
        assert!(result.is_err());
    }

    #[test]
    fn lossy_udp_round_trip() {
        let original = MultitransportRequest {
            request_id: 99,
            requested_protocol: RequestedProtocol::LossyUdp,
            security_cookie: [0xFF; 16],
        };

        let encoded = ironrdp_core::encode_vec(&original).expect("encode");
        let decoded: MultitransportRequest = ironrdp_core::decode(&encoded).expect("decode");
        assert_eq!(decoded, original);
        assert_eq!(decoded.requested_protocol, RequestedProtocol::LossyUdp);
    }
}
