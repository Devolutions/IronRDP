use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduDecode, PduEncode, PduResult};

use crate::{NowHeader, NowMessage, NowMessageClass, NowSessionMessage, NowSessionMessageKind};

/// Message box response; Directly maps to the WinAPI MessageBox function response.
///
/// NOW_PROTO: `response` field from NOW_SESSION_MESSAGE_BOX_RSP_MSG
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NowMsgBoxResponse(u32);

impl NowMsgBoxResponse {
    /// OK
    ///
    /// NOW_PROTO: IDOK
    pub const OK: Self = Self(1);
    /// Cancel
    ///
    /// NOW_PROTO: IDCANCEL
    pub const CANCEL: Self = Self(2);
    /// Abort
    ///
    /// NOW_PROTO: IDABORT
    pub const ABORT: Self = Self(3);
    /// Retry
    ///
    /// NOW_PROTO: IDRETRY
    pub const RETRY: Self = Self(4);
    /// Ignore
    ///
    /// NOW_PROTO: IDIGNORE
    pub const IGNORE: Self = Self(5);
    /// Yes
    ///
    /// NOW_PROTO: IDYES
    pub const YES: Self = Self(6);
    /// No
    ///
    /// NOW_PROTO: IDNO
    pub const NO: Self = Self(7);
    /// Try Again
    ///
    /// NOW_PROTO: IDTRYAGAIN
    pub const TRY_AGAIN: Self = Self(10);
    /// Continue
    ///
    /// NOW_PROTO: IDCONTINUE
    pub const CONTINUE: Self = Self(11);
    /// Timeout
    ///
    /// NOW_PROTO: IDTIMEOUT
    pub const TIMEOUT: Self = Self(32000);

    pub fn new(response: u32) -> Self {
        Self(response)
    }

    pub fn value(&self) -> u32 {
        self.0
    }
}

/// The NOW_SESSION_MSGBOX_RSP_MSG is a message sent in response to NOW_SESSION_MSGBOX_REQ_MSG if
/// the NOW_MSGBOX_FLAG_RESPONSE has been set, and contains the result from the message box dialog.
///
/// NOW_PROTO: NOW_SESSION_MSGBOX_RSP_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowSessionMsgBoxRspMsg {
    request_id: u32,
    response: NowMsgBoxResponse,
}

impl NowSessionMsgBoxRspMsg {
    const NAME: &'static str = "NOW_SESSION_MSGBOX_RSP_MSG";
    const FIXED_PART_SIZE: usize = 8;

    pub fn new(request_id: u32, response: NowMsgBoxResponse) -> Self {
        Self { request_id, response }
    }

    pub fn request_id(&self) -> u32 {
        self.request_id
    }

    pub fn response(&self) -> NowMsgBoxResponse {
        self.response
    }

    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }

    pub(super) fn decode_from_body(_header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let request_id = src.read_u32();
        let response = NowMsgBoxResponse(src.read_u32());

        Ok(Self { request_id, response })
    }
}

impl PduEncode for NowSessionMsgBoxRspMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::SESSION,
            kind: NowSessionMessageKind::MSGBOX_RSP.0,
            flags: 0,
        };

        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.request_id);
        dst.write_u32(self.response.value());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE + self.body_size()
    }
}

impl PduDecode<'_> for NowSessionMsgBoxRspMsg {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowSessionMessageKind(header.kind)) {
            (NowMessageClass::SESSION, NowSessionMessageKind::MSGBOX_RSP) => Self::decode_from_body(header, src),
            _ => Err(unexpected_message_kind_err!(class: header.class.0, kind: header.kind)),
        }
    }
}

impl From<NowSessionMsgBoxRspMsg> for NowMessage {
    fn from(val: NowSessionMsgBoxRspMsg) -> Self {
        NowMessage::Session(NowSessionMessage::MsgBoxRsp(val))
    }
}
