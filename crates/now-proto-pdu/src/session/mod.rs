mod lock;
mod logoff;
mod msg_box_req;
mod msg_box_rsp;

use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduEncode, PduResult};

use crate::NowHeader;

pub use lock::NowSessionLockMsg;
pub use logoff::NowSessionLogoffMsg;
pub use msg_box_req::{NowMessageBoxStyle, NowSessionMsgBoxReqMsg};
pub use msg_box_rsp::{NowMsgBoxResponse, NowSessionMsgBoxRspMsg};

/// Wrapper for the `NOW_SESSION_MSG_CLASS_ID` message class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowSessionMessageKind(pub u8);

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
