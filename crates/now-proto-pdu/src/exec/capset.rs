use bitflags::bitflags;

use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::{DecodeResult, EncodeResult, PduDecode, PduEncode};

use crate::{NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass};

bitflags! {
    /// NOW-PROTO: NOW_EXEC_CAPSET_MSG msgFlags field.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NowExecCapsetFlags: u16 {
        /// Generic "Run" execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_RUN
        const STYLE_RUN = 0x0001;
        /// Generic command execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_CMD
        const STYLE_CMD = 0x0002;
        /// CreateProcess() execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_PROCESS
        const STYLE_PROCESS = 0x0004;
        /// System shell (.sh) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_SHELL
        const STYLE_SHELL = 0x0008;
        /// Windows batch file (.bat) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_BATCH
        const STYLE_BATCH = 0x0010;
        /// Windows PowerShell (.ps1) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_WINPS
        const STYLE_WINPS = 0x0020;
        /// PowerShell 7 (.ps1) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_PWSH
        const STYLE_PWSH = 0x0040;
        /// Applescript (.scpt) execution style.
        ///
        /// NOW-PROTO: NOW_EXEC_STYLE_APPLESCRIPT
        const STYLE_APPLESCRIPT = 0x0080;
    }
}

/// The NOW_EXEC_CAPSET_MSG message is sent to advertise capabilities.
///
/// NOW-PROTO: NOW_EXEC_CAPSET_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecCapsetMsg {
    flags: NowExecCapsetFlags,
}

impl NowExecCapsetMsg {
    const NAME: &'static str = "NOW_EXEC_CAPSET_MSG";

    pub fn new(flags: NowExecCapsetFlags) -> Self {
        Self { flags }
    }

    pub fn flags(&self) -> NowExecCapsetFlags {
        self.flags
    }
}

impl PduEncode for NowExecCapsetMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let header = NowHeader {
            size: 0,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::CAPSET.0,
            flags: self.flags.bits(),
        };

        header.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE
    }
}

impl PduDecode<'_> for NowExecCapsetMsg {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::CAPSET) => Ok(Self {
                flags: NowExecCapsetFlags::from_bits_retain(header.flags),
            }),
            _ => Err(invalid_field_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecCapsetMsg> for NowMessage {
    fn from(msg: NowExecCapsetMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Capset(msg))
    }
}
