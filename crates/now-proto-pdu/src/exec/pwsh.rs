use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduDecode, PduEncode, PduResult};

use crate::{NowExecMessage, NowExecMsgKind, NowExecWinPsFlags, NowHeader, NowMessage, NowMessageClass, NowVarStr};

/// The NOW_EXEC_PWSH_MSG message is used to execute a remote PowerShell 7 (pwsh) command.
///
/// NOW-PROTO: NOW_EXEC_PWSH_MSG
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowExecPwshMsg {
    flags: NowExecWinPsFlags,
    session_id: u32,
    command: NowVarStr,
    execution_policy: NowVarStr,
    configuration_name: NowVarStr,
}

impl NowExecPwshMsg {
    const NAME: &'static str = "NOW_EXEC_PWSH_MSG";
    const FIXED_PART_SIZE: usize = 4;

    pub fn new(session_id: u32, command: NowVarStr) -> PduResult<Self> {
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

    fn ensure_message_size(&self) -> PduResult<()> {
        let _message_size = Self::FIXED_PART_SIZE
            .checked_add(self.command.size())
            .and_then(|size| size.checked_add(self.execution_policy.size()))
            .and_then(|size| size.checked_add(self.configuration_name.size()))
            .ok_or_else(|| invalid_message_err!("size", "message size overflow"))?;

        Ok(())
    }

    #[must_use]
    pub fn with_flags(mut self, flags: NowExecWinPsFlags) -> Self {
        self.flags = flags;
        self
    }

    pub fn with_execution_policy(mut self, execution_policy: NowVarStr) -> PduResult<Self> {
        self.execution_policy = execution_policy;
        self.flags |= NowExecWinPsFlags::EXECUTION_POLICY;

        self.ensure_message_size()?;

        Ok(self)
    }

    pub fn with_configuration_name(mut self, configuration_name: NowVarStr) -> PduResult<Self> {
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

    pub(super) fn decode_from_body(header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
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

impl PduEncode for NowExecPwshMsg {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = NowHeader {
            size: cast_length!("size", self.body_size())?,
            class: NowMessageClass::EXEC,
            kind: NowExecMsgKind::PWSH.0,
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

    // LINTS: see body_size()
    #[allow(clippy::arithmetic_side_effects)]
    fn size(&self) -> usize {
        NowHeader::FIXED_PART_SIZE + self.body_size()
    }
}

impl PduDecode<'_> for NowExecPwshMsg {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = NowHeader::decode(src)?;

        match (header.class, NowExecMsgKind(header.kind)) {
            (NowMessageClass::EXEC, NowExecMsgKind::PWSH) => Self::decode_from_body(header, src),
            _ => Err(invalid_message_err!("type", "invalid message type")),
        }
    }
}

impl From<NowExecPwshMsg> for NowMessage {
    fn from(msg: NowExecPwshMsg) -> Self {
        NowMessage::Exec(NowExecMessage::Pwsh(msg))
    }
}
