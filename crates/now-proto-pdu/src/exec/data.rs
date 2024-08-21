use bitflags::bitflags;

use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::{DecodeResult, EncodeResult, PduDecode, PduEncode};

use crate::{NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass, NowVarBuf};

bitflags! {
    /// NOW-PROTO: NOW_EXEC_DATA_MSG flags field.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NowExecDataFlags: u16 {
        /// This is the first data message.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_DATA_FIRST
        const FIRST = 0x0001;
        /// This is the last data message, the command completed execution.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_DATA_LAST
        const LAST = 0x0002;
        /// The data is from the standard input.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_DATA_STDIN
        const STDIN = 0x0004;
        /// The data is from the standard output.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_DATA_STDOUT
        const STDOUT = 0x0008;
        /// The data is from the standard error.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_DATA_STDERR
        const STDERR = 0x0010;
    }
}

/// The NOW_EXEC_DATA_MSG message is used to send input/output data as part of a remote execution.
///
/// NOW-PROTO: NOW_EXEC_DATA_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecDataMsg {
    flags: NowExecDataFlags,
    session_id: u32,
    data: NowVarBuf,
}

impl NowExecDataMsg {
    const NAME: &'static str = "NOW_EXEC_DATA_MSG";
    const FIXED_PART_SIZE: usize = 4;

    pub fn new(flags: NowExecDataFlags, session_id: u32, data: NowVarBuf) -> Self {
        Self {
            flags,
            session_id,
            data,
        }
    }

    pub fn flags(&self) -> NowExecDataFlags {
        self.flags
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn data(&self) -> &NowVarBuf {
        &self.data
    }

    // LINTS: Overall message size always fits into usize; VarBuf size always a few powers of 2 less
    // than u32::MAX, therefore it fits into usize
    #[allow(clippy::arithmetic_side_effects)]
    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data.size()
    }

    pub(super) fn decode_from_body(header: NowHeader, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = NowExecDataFlags::from_bits_retain(header.flags);
        let session_id = src.read_u32();
        let data = NowVarBuf::decode(src)?;

        Ok(Self {
            flags,
            session_id,
            data,
        })
    }
}

impl PduEncode for NowExecDataMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::DATA.0,
            flags: self.flags.bits(),
        };

        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.session_id);
        self.data.encode(dst)?;

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

impl PduDecode<'_> for NowExecDataMsg {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::DATA) => Self::decode_from_body(header, src),
            _ => Err(invalid_field_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecDataMsg> for NowMessage {
    fn from(msg: NowExecDataMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Data(msg))
    }
}
