use crate::{
    ReadCursor, WriteCursor,
    NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass, NowStatus, PduDecode, PduEncode, PduResult,
};

/// The NOW_EXEC_CANCEL_RSP_MSG message is used to respond to a remote execution cancel request.
///
/// NOW_PROTO: NOW_EXEC_CANCEL_RSP_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecCancelRspMsg {
    session_id: u32,
    status: NowStatus,
}

impl NowExecCancelRspMsg {
    const NAME: &'static str = "NOW_EXEC_CANCEL_RSP_MSG";
    const FIXED_PART_SIZE: usize = 4;

    pub fn new(session_id: u32, status: NowStatus) -> Self {
        Self { session_id, status }
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn status(&self) -> &NowStatus {
        &self.status
    }

    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.status.size()
    }

    pub(super) fn decode_from_body(_header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let session_id = src.read_u32();
        let status = NowStatus::decode(src)?;

        Ok(Self { session_id, status })
    }
}

impl PduEncode for NowExecCancelRspMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::CANCEL_RSP.0,
            flags: 0,
        };

        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.session_id);
        self.status.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE + self.body_size()
    }
}

impl PduDecode<'_> for NowExecCancelRspMsg {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::CANCEL_RSP) => Self::decode_from_body(header, src),
            _ => Err(invalid_message_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecCancelRspMsg> for NowMessage {
    fn from(msg: NowExecCancelRspMsg) -> Self {
        NowMessage::Exec(NowExecMessage::CancelRsp(msg))
    }
}
