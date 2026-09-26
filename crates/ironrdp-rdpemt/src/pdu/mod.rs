//! RDPEMT tunnel PDU definitions per MS-RDPEMT Section 2.2.
//!
//! Three PDU types share a common `TunnelHeader`:
//!
//! - [`TunnelCreateRequest`]: client → server tunnel binding (Section 2.2.2.1)
//! - [`TunnelCreateResponse`]: server → client confirmation (Section 2.2.2.2)
//! - [`TunnelData`]: bidirectional data transport (Section 2.2.2.3)
//!
//! The top-level [`TunnelPdu`] enum dispatches decoding based on the Action
//! nibble in byte 0 of the tunnel header.

pub mod create_request;
pub mod create_response;
pub mod data;
pub mod header;
pub mod subheader;

pub use create_request::{SECURITY_COOKIE_LEN, TunnelCreateRequest};
pub use create_response::TunnelCreateResponse;
pub use data::TunnelData;
pub use header::{TunnelAction, TunnelHeader};
use ironrdp_core::{Decode, DecodeResult, ReadCursor, UnexpectedMessageTypeErr as _};
pub use subheader::{SubHeaderType, TunnelSubHeader};

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
/// Discriminated union of all RDPEMT PDU types.
///
/// Decoded by reading the Action nibble from byte 0 and dispatching
/// to the appropriate variant's decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelPdu {
    /// Client → server tunnel creation request.
    CreateRequest(TunnelCreateRequest),
    /// Server → client tunnel creation response.
    CreateResponse(TunnelCreateResponse),
    /// Bidirectional higher-layer data.
    Data(TunnelData),
}

impl Decode<'_> for TunnelPdu {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        // Peek at byte 0 to determine the action without consuming it,
        // since the individual PDU decoders expect the full wire bytes
        // starting from the header.
        ironrdp_core::ensure_size!(in: src, size: 1);
        let byte0 = src.remaining()[0];
        let action_raw = byte0 & 0x0F;

        let action = TunnelAction::from_u8(action_raw).ok_or_else(|| {
            ironrdp_core::DecodeError::unexpected_message_type("TunnelPdu", action_raw, Some(src.pos()))
        })?;

        match action {
            TunnelAction::CreateRequest => TunnelCreateRequest::decode(src).map(TunnelPdu::CreateRequest),
            TunnelAction::CreateResponse => TunnelCreateResponse::decode(src).map(TunnelPdu::CreateResponse),
            TunnelAction::Data => TunnelData::decode(src).map(TunnelPdu::Data),
        }
    }
}
