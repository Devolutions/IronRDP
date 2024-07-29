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
pub(crate) struct NowSystemMessageKind(u8);

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

#[cfg(all(test, feature = "std"))]
mod tests {
    use alloc::string::ToString;

    use super::*;
    use crate::{test_utils::now_msg_roundtrip, NowVarStr};

    use expect_test::expect;

    #[test]
    fn roundtip_system_shutdown() {
        now_msg_roundtrip(
            NowSystemShutdownMsg {
                flags: NowSystemShutdownFlags::FORCE,
                message: NowVarStr::new("hello".to_string()).unwrap(),
                timeout: 0x12345678,
            },
            expect!["[0B, 00, 00, 00, 11, 03, 01, 00, 78, 56, 34, 12, 05, 68, 65, 6C, 6C, 6F, 00]"],
        );
    }
}
