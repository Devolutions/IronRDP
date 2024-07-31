mod shutdown;

use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduEncode, PduResult};

use crate::NowHeader;

pub use shutdown::{NowSystemShutdownFlags, NowSystemShutdownMsg};

// Wrapper for the `NOW_SYSTEM_MSG_CLASS_ID` message class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NowSystemMessage {
    Shutdown(NowSystemShutdownMsg),
}

impl NowSystemMessage {
    const NAME: &'static str = "NOW_SYSTEM_MSG";

    pub fn decode_from_body(header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        match NowSystemMessageKind(header.kind) {
            NowSystemMessageKind::SHUTDOWN => Ok(Self::Shutdown(NowSystemShutdownMsg::decode_from_body(header, src)?)),
            _ => Err(unexpected_message_kind_err!(class: header.class.0, kind: header.kind)),
        }
    }
}

impl PduEncode for NowSystemMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            Self::Shutdown(msg) => msg.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::Shutdown(msg) => msg.size(),
        }
    }
}

/// NOW-PROTO: NOW_SYSTEM_INFO_*_ID
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NowSystemMessageKind(pub u8);

impl NowSystemMessageKind {
    // TODO: NOW_SYSTEM_INFO_REQ_ID/NOW_SYSTEM_INFO_RSP_ID when will be added to the protocol
    // specification.

    // /// NOW-PROTO: NOW_SYSTEM_INFO_REQ_ID
    // pub const INFO_REQ: Self = Self(0x01);
    // /// NOW-PROTO: NOW_SYSTEM_INFO_RSP_ID
    // pub const INFO_RSP: Self = Self(0x02);
    /// NOW-PROTO: NOW_SYSTEM_SHUTDOWN_ID
    pub const SHUTDOWN: Self = Self(0x03);
}
