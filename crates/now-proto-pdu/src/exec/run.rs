use crate::{
    ReadCursor, WriteCursor,
    NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass, NowVarStr, PduDecode, PduEncode, PduResult,
};

/// The NOW_EXEC_RUN_MSG message is used to send a run request. This request type maps to starting
/// a program by using the “Run” menu on operating systems (the Start Menu on Windows, the Dock on
/// macOS etc.). The execution of programs started with NOW_EXEC_RUN_MSG is not followed and does
/// not send back the output.
///
/// NOW_PROTO: NOW_EXEC_RUN_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecRunMsg {
    session_id: u32,
    command: NowVarStr,
}

impl NowExecRunMsg {
    const NAME: &'static str = "NOW_EXEC_RUN_MSG";
    const FIXED_PART_SIZE: usize = 4;

    pub fn new(session_id: u32, command: NowVarStr) -> Self {
        Self { session_id, command }
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn command(&self) -> &NowVarStr {
        &self.command
    }

    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.command.size()
    }

    pub(super) fn decode_from_body(_header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let session_id = src.read_u32();
        let command = NowVarStr::decode(src)?;

        Ok(Self { session_id, command })
    }
}

impl PduEncode for NowExecRunMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::RUN.0,
            flags: 0,
        };

        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.session_id);
        self.command.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE + self.body_size()
    }
}

impl PduDecode<'_> for NowExecRunMsg {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::RUN) => Self::decode_from_body(header, src),
            _ => Err(invalid_message_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecRunMsg> for NowMessage {
    fn from(msg: NowExecRunMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Run(msg))
    }
}
