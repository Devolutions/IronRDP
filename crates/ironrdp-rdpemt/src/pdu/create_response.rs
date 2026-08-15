//! RDP_TUNNEL_CREATERESPONSE: server → client tunnel confirmation.
//!
//! MS-RDPEMT Section 2.2.2.2.
//!
//! Wire layout (8 bytes total):
//! ```text
//! Bytes 0-3: TunnelHeader (Action=0x1, PayloadLength=0x0004, HeaderLength=0x04)
//! Bytes 4-7: HrResponse (u32 LE, HRESULT; 0x00000000 = S_OK)
//! ```

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, InvalidFieldErr as _, ReadCursor, UnexpectedMessageTypeErr as _,
    WriteCursor,
};

use crate::pdu::header::{TunnelAction, TunnelHeader};

/// Payload size: HrResponse(4) = 4.
const PAYLOAD_SIZE: usize = 4;

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Tunnel Create Response PDU.
///
/// Sent by the server to confirm or reject tunnel creation.
/// An HrResponse of `S_OK` (0x00000000) means success; any other
/// value indicates rejection and the client MUST disconnect
/// (MS-RDPEMT Section 3.3.5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelCreateResponse {
    /// HRESULT code indicating success or failure.
    pub hr_response: u32,
}

impl TunnelCreateResponse {
    /// S_OK: tunnel created successfully.
    pub const S_OK: u32 = 0x0000_0000;

    const NAME: &'static str = "RDP_TUNNEL_CREATERESPONSE";

    /// Total wire size: 4 (header) + 4 (payload) = 8 bytes.
    const WIRE_SIZE: usize = TunnelHeader::MIN_SIZE + PAYLOAD_SIZE;

    /// HRESULT severity bit (bit 31). Clear means success, set means failure.
    ///
    /// MS-RDPEMT 3.1.5.5 / 3.3.5.1 require a successful HRESULT, not
    /// specifically `S_OK`; this matches the Win32 `SUCCEEDED(hr)` macro
    /// rather than an exact-equality check, so any success code a non-Windows
    /// peer might send is still recognized.
    const SEVERITY_BIT: u32 = 0x8000_0000;

    /// Whether this response indicates successful tunnel creation.
    pub fn is_success(&self) -> bool {
        self.hr_response & Self::SEVERITY_BIT == 0
    }
}

impl Encode for TunnelCreateResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ironrdp_core::ensure_size!(in: dst, size: self.size());

        let header = TunnelHeader {
            action: TunnelAction::CreateResponse,
            #[expect(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                reason = "PAYLOAD_SIZE is 4; fits in u16"
            )]
            payload_length: PAYLOAD_SIZE as u16,
            header_length: 4,
            sub_headers: Vec::new(),
        };
        header.encode(dst)?;

        dst.write_u32(self.hr_response);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::WIRE_SIZE
    }
}

impl Decode<'_> for TunnelCreateResponse {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = TunnelHeader::decode(src)?;

        if header.action != TunnelAction::CreateResponse {
            return Err(ironrdp_core::DecodeError::unexpected_message_type(
                Self::NAME,
                header.action.to_u8(),
            ));
        }

        if header.header_length != 4 {
            return Err(ironrdp_core::DecodeError::invalid_field(
                Self::NAME,
                "HeaderLength",
                "must be 4 for CreateResponse",
            ));
        }

        // PayloadLength is fixed at 4 for this PDU (MS-RDPEMT 2.2.2.2);
        // reject a declared length that disagrees with the fixed body this
        // decoder actually reads, since ironrdp_core::decode does not
        // require the cursor to be fully consumed.
        if usize::from(header.payload_length) != PAYLOAD_SIZE {
            return Err(ironrdp_core::DecodeError::invalid_field(
                Self::NAME,
                "PayloadLength",
                "must be 4 for CreateResponse",
            ));
        }

        ironrdp_core::ensure_size!(in: src, size: PAYLOAD_SIZE);
        let hr_response = src.read_u32();

        Ok(Self { hr_response })
    }
}
