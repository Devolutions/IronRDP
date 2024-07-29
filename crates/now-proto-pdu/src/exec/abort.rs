use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduDecode, PduEncode, PduResult};

use crate::{NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass, NowStatus};

/// The NOW_EXEC_ABORT_MSG message is used to abort a remote execution immediately due to an
/// unrecoverable error. This message can be sent at any time without an explicit response message.
/// The session is considered aborted as soon as this message is sent.
///
/// NOW-PROTO: NOW_EXEC_ABORT_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecAbortMsg {
    session_id: u32,
    status: NowStatus,
}

impl NowExecAbortMsg {
    const NAME: &'static str = "NOW_EXEC_ABORT_MSG";
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

impl PduEncode for NowExecAbortMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::ABORT.0,
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

impl PduDecode<'_> for NowExecAbortMsg {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::ABORT) => Self::decode_from_body(header, src),
            _ => Err(invalid_message_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecAbortMsg> for NowMessage {
    fn from(msg: NowExecAbortMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Abort(msg))
    }
}
