//! RDP_TUNNEL_CREATEREQUEST: client → server tunnel binding.
//!
//! MS-RDPEMT Section 2.2.2.1.
//!
//! Wire layout (28 bytes total):
//! ```text
//! Bytes 0-3:   TunnelHeader (Action=0x0, PayloadLength=0x0018, HeaderLength=0x04)
//! Bytes 4-7:   RequestID (u32 LE)
//! Bytes 8-11:  Reserved (u32 LE, MUST be 0)
//! Bytes 12-27: SecurityCookie (16 bytes)
//! ```

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, InvalidFieldErr as _, ReadCursor, UnexpectedMessageTypeErr as _,
    WriteCursor,
};

use crate::pdu::header::{TunnelAction, TunnelHeader};

/// Length of the security cookie (MS-RDPEMT Section 2.2.2.1).
pub const SECURITY_COOKIE_LEN: usize = 16;

/// Payload size: RequestID(4) + Reserved(4) + SecurityCookie(16) = 24.
const PAYLOAD_SIZE: usize = 4 + 4 + SECURITY_COOKIE_LEN;

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Tunnel Create Request PDU.
///
/// Sent by the client after the TLS handshake to bind this UDP tunnel
/// to a main RDP session. The `request_id` and `security_cookie` must
/// match the Initiate Multitransport Request PDU sent by the server
/// over the main TCP connection (MS-RDPBCGR Section 2.2.15.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelCreateRequest {
    /// Request ID from the Initiate Multitransport Request PDU.
    pub request_id: u32,
    /// 16-byte security cookie from the Initiate Multitransport Request PDU.
    pub security_cookie: [u8; SECURITY_COOKIE_LEN],
}

impl TunnelCreateRequest {
    const NAME: &'static str = "RDP_TUNNEL_CREATEREQUEST";

    /// Total wire size: 4 (header) + 24 (payload) = 28 bytes.
    const WIRE_SIZE: usize = TunnelHeader::MIN_SIZE + PAYLOAD_SIZE;
}

impl Encode for TunnelCreateRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_size!(in: dst, size: self.size());

        let header = TunnelHeader {
            action: TunnelAction::CreateRequest,
            #[expect(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                reason = "PAYLOAD_SIZE is 24; fits in u16"
            )]
            payload_length: PAYLOAD_SIZE as u16,
            header_length: 4,
            sub_headers: Vec::new(),
        };
        header.encode(dst)?;

        dst.write_u32(self.request_id);
        dst.write_u32(0); // Reserved
        dst.write_slice(&self.security_cookie);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::WIRE_SIZE
    }
}

impl Decode<'_> for TunnelCreateRequest {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = TunnelHeader::decode(src)?;

        if header.action != TunnelAction::CreateRequest {
            return Err(ironrdp_core::DecodeError::unexpected_message_type(
                Self::NAME,
                header.action.to_u8(),
            ));
        }

        if header.header_length != 4 {
            return Err(ironrdp_core::DecodeError::invalid_field(
                Self::NAME,
                "HeaderLength",
                "must be 4 for CreateRequest",
            ));
        }

        // PayloadLength is fixed at 24 for this PDU (MS-RDPEMT 2.2.2.1);
        // reject a declared length that disagrees with the fixed body this
        // decoder actually reads, since ironrdp_core::decode does not
        // require the cursor to be fully consumed.
        if usize::from(header.payload_length) != PAYLOAD_SIZE {
            return Err(ironrdp_core::DecodeError::invalid_field(
                Self::NAME,
                "PayloadLength",
                "must be 24 for CreateRequest",
            ));
        }

        ironrdp_core::ensure_size!(in: src, size: PAYLOAD_SIZE);

        let request_id = src.read_u32();
        let _reserved = src.read_u32();
        let security_cookie: [u8; SECURITY_COOKIE_LEN] = src.read_array();

        Ok(Self {
            request_id,
            security_cookie,
        })
    }
}
