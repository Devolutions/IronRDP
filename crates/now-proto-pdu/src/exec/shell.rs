use ironrdp_core::{
    cast_length, ensure_fixed_part_size, invalid_field_err, DecodeResult, EncodeResult, ReadCursor, WriteCursor,
};
use ironrdp_pdu::{Decode, Encode};

use crate::{NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass, NowVarStr};

/// The NOW_EXEC_SHELL_MSG message is used to execute a remote shell command.
///
/// NOW-PROTO: NOW_EXEC_SHELL_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecShellMsg {
    session_id: u32,
    command: NowVarStr,
    shell: NowVarStr,
}

impl NowExecShellMsg {
    const NAME: &'static str = "NOW_EXEC_SHELL_MSG";
    const FIXED_PART_SIZE: usize = 4;

    pub fn new(session_id: u32, command: NowVarStr, shell: NowVarStr) -> DecodeResult<Self> {
        let msg = Self {
            session_id,
            command,
            shell,
        };

        msg.ensure_message_size()?;

        Ok(msg)
    }

    fn ensure_message_size(&self) -> DecodeResult<()> {
        let _message_size = Self::FIXED_PART_SIZE
            .checked_add(self.command.size())
            .and_then(|size| size.checked_add(self.shell.size()))
            .ok_or_else(|| invalid_field_err!("size", "message size overflow"))?;

        Ok(())
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn command(&self) -> &NowVarStr {
        &self.command
    }

    pub fn shell(&self) -> &NowVarStr {
        &self.shell
    }

    // LINTS: Overall message size is validated in the constructor/decode method
    #[allow(clippy::arithmetic_side_effects)]
    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.command.size() + self.shell.size()
    }

    pub(super) fn decode_from_body(_header: NowHeader, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let session_id = src.read_u32();
        let command = NowVarStr::decode(src)?;
        let shell = NowVarStr::decode(src)?;

        let msg = Self {
            session_id,
            command,
            shell,
        };

        msg.ensure_message_size()?;

        Ok(msg)
    }
}

impl Encode for NowExecShellMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::SHELL.0,
            flags: 0,
        };

        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.session_id);
        self.command.encode(dst)?;
        self.shell.encode(dst)?;

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

impl Decode<'_> for NowExecShellMsg {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::SHELL) => Self::decode_from_body(header, src),
            _ => Err(invalid_field_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecShellMsg> for NowMessage {
    fn from(msg: NowExecShellMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Shell(msg))
    }
}
