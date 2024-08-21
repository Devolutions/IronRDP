use bitflags::bitflags;

use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::{DecodeResult, EncodeResult, PduDecode, PduEncode};

use crate::{NowExecMessage, NowExecMsgKind, NowHeader, NowMessage, NowMessageClass, NowVarStr};

bitflags! {
    /// NOW-PROTO: NOW_EXEC_WINPS_MSG msgFlags field.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NowExecWinPsFlags: u16 {
        /// PowerShell -NoLogo option.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_NO_LOGO
        const NO_LOGO = 0x0001;
        /// PowerShell -NoExit option.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_NO_EXIT
        const NO_EXIT = 0x0002;
        /// PowerShell -Sta option.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_STA
        const STA = 0x0004;
        /// PowerShell -Mta option
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_MTA
        const MTA = 0x0008;
        /// PowerShell -NoProfile option.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_NO_PROFILE
        const NO_PROFILE = 0x0010;
        /// PowerShell -NonInteractive option.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_NON_INTERACTIVE
        const NON_INTERACTIVE = 0x0020;
        /// The PowerShell -ExecutionPolicy parameter is specified with value in
        /// executionPolicy field.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_EXECUTION_POLICY
        const EXECUTION_POLICY = 0x0040;
        /// The PowerShell -ConfigurationName parameter is specified with value in
        /// configurationName field.
        ///
        /// NOW-PROTO: NOW_EXEC_FLAG_PS_CONFIGURATION_NAME
        const CONFIGURATION_NAME = 0x0080;
    }
}

/// The NOW_EXEC_WINPS_MSG message is used to execute a remote Windows PowerShell (powershell.exe) command.
///
/// NOW-PROTO: NOW_EXEC_WINPS_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecWinPsMsg {
    flags: NowExecWinPsFlags,
    session_id: u32,
    command: NowVarStr,
    execution_policy: NowVarStr,
    configuration_name: NowVarStr,
}

impl NowExecWinPsMsg {
    const NAME: &'static str = "NOW_EXEC_WINPS_MSG";
    const FIXED_PART_SIZE: usize = 4;

    pub fn new(session_id: u32, command: NowVarStr) -> DecodeResult<Self> {
        let msg = Self {
            session_id,
            command,
            flags: NowExecWinPsFlags::empty(),
            execution_policy: NowVarStr::empty(),
            configuration_name: NowVarStr::empty(),
        };

        msg.ensure_message_size()?;

        Ok(msg)
    }

    fn ensure_message_size(&self) -> DecodeResult<()> {
        let _message_size = Self::FIXED_PART_SIZE
            .checked_add(self.command.size())
            .and_then(|size| size.checked_add(self.execution_policy.size()))
            .and_then(|size| size.checked_add(self.configuration_name.size()))
            .ok_or_else(|| invalid_field_err!("size", "message size overflow"))?;

        Ok(())
    }

    #[must_use]
    pub fn with_flags(mut self, flags: NowExecWinPsFlags) -> Self {
        self.flags = flags;
        self
    }

    pub fn with_execution_policy(mut self, execution_policy: NowVarStr) -> DecodeResult<Self> {
        self.execution_policy = execution_policy;
        self.flags |= NowExecWinPsFlags::EXECUTION_POLICY;

        self.ensure_message_size()?;

        Ok(self)
    }

    pub fn with_configuration_name(mut self, configuration_name: NowVarStr) -> DecodeResult<Self> {
        self.configuration_name = configuration_name;
        self.flags |= NowExecWinPsFlags::CONFIGURATION_NAME;

        self.ensure_message_size()?;

        Ok(self)
    }

    pub fn flags(&self) -> NowExecWinPsFlags {
        self.flags
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    pub fn command(&self) -> &NowVarStr {
        &self.command
    }

    pub fn execution_policy(&self) -> Option<&NowVarStr> {
        if self.flags.contains(NowExecWinPsFlags::EXECUTION_POLICY) {
            Some(&self.execution_policy)
        } else {
            None
        }
    }

    pub fn configuration_name(&self) -> Option<&NowVarStr> {
        if self.flags.contains(NowExecWinPsFlags::CONFIGURATION_NAME) {
            Some(&self.configuration_name)
        } else {
            None
        }
    }

    // LINTS: Overall message size is validated in the constructor/decode method
    #[allow(clippy::arithmetic_side_effects)]
    fn body_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.command.size() + self.execution_policy.size() + self.configuration_name.size()
    }

    pub(super) fn decode_from_body(header: NowHeader, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let flags = NowExecWinPsFlags::from_bits_retain(header.flags);
        let session_id = src.read_u32();
        let command = NowVarStr::decode(src)?;
        let execution_policy = NowVarStr::decode(src)?;
        let configuration_name = NowVarStr::decode(src)?;

        let msg = Self {
            flags,
            session_id,
            command,
            execution_policy,
            configuration_name,
        };

        msg.ensure_message_size()?;

        Ok(msg)
    }
}

impl PduEncode for NowExecWinPsMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::WINPS.0,
            flags: self.flags.bits(),
        };

        header.encode(dst)?;

        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.session_id);
        self.command.encode(dst)?;
        self.execution_policy.encode(dst)?;
        self.configuration_name.encode(dst)?;

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

impl PduDecode<'_> for NowExecWinPsMsg {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::WINPS) => Self::decode_from_body(header, src),
            _ => Err(invalid_field_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecWinPsMsg> for NowMessage {
    fn from(msg: NowExecWinPsMsg) -> Self {
        NowMessage::Exec(NowExecMessage::WinPs(msg))
    }
}
