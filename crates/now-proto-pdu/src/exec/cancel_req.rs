use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduDecode, PduEncode, PduResult};

use crate::{NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass};

/// The NOW_EXEC_CANCEL_REQ_MSG message is used to cancel a remote execution session.
///
/// NOW-PROTO: NOW_EXEC_CANCEL_REQ_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecCancelReqMsg {
    session_id: u32,
}

impl NowExecCancelReqMsg {
    const NAME: &'static str = "NOW_EXEC_CANCEL_REQ_MSG";
    const FIXED_PART_SIZE: usize = 4;

    pub fn new(session_id: u32) -> Self {
        Self { session_id }
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub(super) fn decode_from_body(_header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let session_id = src.read_u32();

        Ok(Self { session_id })
    }
}

impl PduEncode for NowExecCancelReqMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = NowHeader {
            size: u32::try_from(Self::FIXED_PART_SIZE).unwrap(),
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::CANCEL_REQ.0,
            flags: 0,
        };

        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.session_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE + Self::FIXED_PART_SIZE
    }
}

impl PduDecode<'_> for NowExecCancelReqMsg {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::CANCEL_REQ) => Self::decode_from_body(header, src),
            _ => Err(invalid_field_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecCancelReqMsg> for NowMessage {
    fn from(msg: NowExecCancelReqMsg) -> Self {
        NowMessage::Exec(NowExecMessage::CancelReq(msg))
    }
}
