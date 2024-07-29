use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduDecode, PduEncode, PduResult};

use crate::{NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass, NowVarStr};

/// The NOW_EXEC_PROCESS_MSG message is used to send a Windows CreateProcess() request.
///
/// NOW-PROTO: NOW_EXEC_PROCESS_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecProcessMsg {
    session_id: u32,
    filename: NowVarStr,
    parameters: NowVarStr,
    directory: NowVarStr,
}

impl NowExecProcessMsg {
    const NAME: &'static str = "NOW_EXEC_PROCESS_MSG";
    const FIXED_PART_SIZE: usize = 4;

    pub fn new(session_id: u32, filename: NowVarStr, parameters: NowVarStr, directory: NowVarStr) -> Self {
        Self {
            session_id,
            filename,
            parameters,
            directory,
        }
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn filename(&self) -> &NowVarStr {
        &self.filename
    }

    pub fn parameters(&self) -> &NowVarStr {
        &self.parameters
    }

    pub fn directory(&self) -> &NowVarStr {
        &self.directory
    }

    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.filename.size() + self.parameters.size() + self.directory.size()
    }

    pub(super) fn decode_from_body(_header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let session_id = src.read_u32();
        let filename = NowVarStr::decode(src)?;
        let parameters = NowVarStr::decode(src)?;
        let directory = NowVarStr::decode(src)?;

        Ok(Self {
            session_id,
            filename,
            parameters,
            directory,
        })
    }
}

impl PduEncode for NowExecProcessMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::PROCESS.0,
            flags: 0,
        };

        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.session_id);
        self.filename.encode(dst)?;
        self.parameters.encode(dst)?;
        self.directory.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE + self.body_size()
    }
}

impl PduDecode<'_> for NowExecProcessMsg {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::PROCESS) => Self::decode_from_body(header, src),
            _ => Err(invalid_message_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecProcessMsg> for NowMessage {
    fn from(msg: NowExecProcessMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Process(msg))
    }
}
