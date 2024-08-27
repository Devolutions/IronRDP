use ironrdp_core::{
    cast_length, ensure_fixed_part_size, invalid_field_err, DecodeResult, EncodeResult, ReadCursor, WriteCursor,
};
use ironrdp_core::{Decode, Encode};

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

    pub fn new(
        session_id: u32,
        filename: NowVarStr,
        parameters: NowVarStr,
        directory: NowVarStr,
    ) -> DecodeResult<Self> {
        let msg = Self {
            session_id,
            filename,
            parameters,
            directory,
        };

        msg.ensure_message_size()?;

        Ok(msg)
    }

    fn ensure_message_size(&self) -> DecodeResult<()> {
        let _message_size = Self::FIXED_PART_SIZE
            .checked_add(self.filename.size())
            .and_then(|size| size.checked_add(self.parameters.size()))
            .and_then(|size| size.checked_add(self.directory.size()))
            .ok_or_else(|| invalid_field_err!("size", "message size overflow"))?;

        Ok(())
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

    // LINTS: Overall message size is validated in the constructor/decode method
    #[allow(clippy::arithmetic_side_effects)]
    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.filename.size() + self.parameters.size() + self.directory.size()
    }

    pub(super) fn decode_from_body(_header: NowHeader, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let session_id = src.read_u32();
        let filename = NowVarStr::decode(src)?;
        let parameters = NowVarStr::decode(src)?;
        let directory = NowVarStr::decode(src)?;

        let msg = Self {
            session_id,
            filename,
            parameters,
            directory,
        };

        msg.ensure_message_size()?;

        Ok(msg)
    }
}

impl Encode for NowExecProcessMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
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

    // LINTS: See body_size()
    #[allow(clippy::arithmetic_side_effects)]
    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE + self.body_size()
    }
}

impl Decode<'_> for NowExecProcessMsg {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::PROCESS) => Self::decode_from_body(header, src),
            _ => Err(invalid_field_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecProcessMsg> for NowMessage {
    fn from(msg: NowExecProcessMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Process(msg))
    }
}
