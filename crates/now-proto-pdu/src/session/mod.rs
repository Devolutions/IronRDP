mod lock;
mod logoff;
mod msg_box_req;
mod msg_box_rsp;

use crate::{
    ReadCursor, WriteCursor,
    NowHeader, PduEncode, PduResult,
};

pub use lock::NowSessionLockMsg;
pub use logoff::NowSessionLogoffMsg;
pub use msg_box_req::{NowMessageBoxStyle, NowSessionMsgBoxReqMsg};
pub use msg_box_rsp::{NowMsgBoxResponse, NowSessionMsgBoxRspMsg};

/// Wrapper for the `NOW_SESSION_MSG_CLASS_ID` message class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NowSessionMessageKind(pub u8);

impl NowSessionMessageKind {
    /// NOW-PROTO: NOW_SESSION_LOCK_MSG_ID
    pub const LOCK: Self = Self(0x01);
    /// NOW-PROTO: NOW_SESSION_LOGOFF_MSG_ID
    pub const LOGOFF: Self = Self(0x02);
    /// NOW-PROTO: NOW_SESSION_MSGBOX_REQ_MSG_ID
    pub const MSGBOX_REQ: Self = Self(0x03);
    /// NOW-PROTO: NOW_SESSION_MSGBOX_RSP_MSG_ID
    pub const MSGBOX_RSP: Self = Self(0x04);
}

// Wrapper for the `NOW_SESSION_MSG_CLASS_ID` message class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NowSessionMessage {
    Lock(NowSessionLockMsg),
    Logoff(NowSessionLogoffMsg),
    MsgBoxReq(NowSessionMsgBoxReqMsg),
    MsgBoxRsp(NowSessionMsgBoxRspMsg),
}

impl NowSessionMessage {
    const NAME: &'static str = "NOW_SESSION_MSG";

    pub fn decode_from_body(header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        match NowSessionMessageKind(header.kind) {
            NowSessionMessageKind::LOCK => Ok(Self::Lock(NowSessionLockMsg::default())),
            NowSessionMessageKind::LOGOFF => Ok(Self::Logoff(NowSessionLogoffMsg::default())),
            NowSessionMessageKind::MSGBOX_REQ => {
                Ok(Self::MsgBoxReq(NowSessionMsgBoxReqMsg::decode_from_body(header, src)?))
            }
            NowSessionMessageKind::MSGBOX_RSP => {
                Ok(Self::MsgBoxRsp(NowSessionMsgBoxRspMsg::decode_from_body(header, src)?))
            }
            _ => Err(unexpected_message_kind_err!(class: header.class.0, kind: header.kind)),
        }
    }
}

impl PduEncode for NowSessionMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            Self::Lock(msg) => msg.encode(dst),
            Self::Logoff(msg) => msg.encode(dst),
            Self::MsgBoxReq(msg) => msg.encode(dst),
            Self::MsgBoxRsp(msg) => msg.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::Lock(msg) => msg.size(),
            Self::Logoff(msg) => msg.size(),
            Self::MsgBoxReq(msg) => msg.size(),
            Self::MsgBoxRsp(msg) => msg.size(),
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use alloc::string::ToString;

    use super::*;
    use crate::{test_utils::now_msg_roundtrip, NowVarStr};

    use expect_test::expect;
    #[test]
    fn roundtrip_session_lock() {
        now_msg_roundtrip(
            NowSessionLockMsg::default(),
            expect!["[00, 00, 00, 00, 12, 01, 00, 00]"],
        );
    }

    #[test]
    fn roundtrip_session_logoff() {
        now_msg_roundtrip(
            NowSessionLogoffMsg::default(),
            expect!["[00, 00, 00, 00, 12, 02, 00, 00]"],
        );
    }

    #[test]
    fn roundtip_session_msgbox_req() {
        now_msg_roundtrip(
            NowSessionMsgBoxReqMsg::new(
                0x76543210,
                NowVarStr::new("hello".to_string()).unwrap(),
            ).with_response().with_style(NowMessageBoxStyle::ABORT_RETRY_IGNORE)
            .with_title(NowVarStr::new("world".to_string()).unwrap())
            .with_timeout(3),
            expect!["[1A, 00, 00, 00, 12, 03, 0F, 00, 10, 32, 54, 76, 02, 00, 00, 00, 03, 00, 00, 00, 05, 77, 6F, 72, 6C, 64, 00, 05, 68, 65, 6C, 6C, 6F, 00]"]
        );
    }

    #[test]
    fn roundtrip_session_msgbox_rsp() {
        now_msg_roundtrip(
            NowSessionMsgBoxRspMsg::new(0x01234567, NowMsgBoxResponse::RETRY),
            expect!["[08, 00, 00, 00, 12, 04, 00, 00, 67, 45, 23, 01, 04, 00, 00, 00]"],
        );
    }
}
