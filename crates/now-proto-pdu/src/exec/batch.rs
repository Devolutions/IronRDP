use ironrdp_core::{
    cast_length, ensure_fixed_part_size, invalid_field_err, Decode, DecodeResult, Encode, EncodeResult, ReadCursor,
    WriteCursor,
};

use crate::{NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass, NowVarStr};

/// The NOW_EXEC_BATCH_MSG message is used to execute a remote batch command.
///
/// NOW-PROTO: NOW_EXEC_BATCH_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecBatchMsg {
    session_id: u32,
    command: NowVarStr,
}

impl NowExecBatchMsg {
    const NAME: &'static str = "NOW_EXEC_BATCH_MSG";
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

    // LINTS: Overall message size always fits into usize; VarStr size always a few powers of 2 less
    // than u32::MAX, therefore it fits into usize
    #[allow(clippy::arithmetic_side_effects)]
    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.command.size()
    }

    pub(super) fn decode_from_body(_header: NowHeader, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let session_id = src.read_u32();
        let command = NowVarStr::decode(src)?;

        Ok(Self { session_id, command })
    }
}

impl Encode for NowExecBatchMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::BATCH.0,
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

impl Decode<'_> for NowExecBatchMsg {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::BATCH) => Self::decode_from_body(header, src),
            _ => Err(invalid_field_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecBatchMsg> for NowMessage {
    fn from(msg: NowExecBatchMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Batch(msg))
    }
}
