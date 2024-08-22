use ironrdp_core::{
    cast_length, ensure_fixed_part_size, invalid_field_err, DecodeResult, EncodeResult, ReadCursor, WriteCursor,
};
use ironrdp_pdu::{Decode, Encode};

use crate::{NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass, NowVarStr};

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

    pub fn new(session_id: u32, command: NowVarStr) -> DecodeResult<Self> {
        let msg = Self { session_id, command };

        msg.ensure_message_size()?;

        Ok(msg)
    }

    fn ensure_message_size(&self) -> DecodeResult<()> {
        let _message_size = Self::FIXED_PART_SIZE
            .checked_add(self.command.size())
            .ok_or_else(|| invalid_field_err!("size", "message size overflow"))?;

        Ok(())
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn command(&self) -> &NowVarStr {
        &self.command
    }

    // LINTS: Overall message size is validated in the constructor/decode method
    #[allow(clippy::arithmetic_side_effects)]
    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.command.size()
    }

    pub(super) fn decode_from_body(_header: NowHeader, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let session_id = src.read_u32();
        let command = NowVarStr::decode(src)?;

        let msg = Self { session_id, command };

        msg.ensure_message_size()?;

        Ok(msg)
    }
}

impl Encode for NowExecRunMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
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

    // LINTS: See body_size()
    #[allow(clippy::arithmetic_side_effects)]
    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE + self.body_size()
    }
}

impl Decode<'_> for NowExecRunMsg {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::RUN) => Self::decode_from_body(header, src),
            _ => Err(invalid_field_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecRunMsg> for NowMessage {
    fn from(msg: NowExecRunMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Run(msg))
    }
}
