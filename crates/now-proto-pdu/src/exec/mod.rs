mod abort;
mod batch;
mod cancel_req;
mod cancel_rsp;
mod capset;
mod cmd;
mod data;
mod process;
mod pwsh;
mod result;
mod run;
mod shell;
mod win_ps;

use ironrdp_core::{invalid_field_err, DecodeResult, EncodeResult, ReadCursor, WriteCursor};
use ironrdp_pdu::Encode;

use crate::NowHeader;

pub use abort::NowExecAbortMsg;
pub use batch::NowExecBatchMsg;
pub use cancel_req::NowExecCancelReqMsg;
pub use cancel_rsp::NowExecCancelRspMsg;
pub use capset::{NowExecCapsetFlags, NowExecCapsetMsg};
pub use data::{NowExecDataFlags, NowExecDataMsg};
pub use process::NowExecProcessMsg;
pub use pwsh::NowExecPwshMsg;
pub use result::NowExecResultMsg;
pub use run::NowExecRunMsg;
pub use shell::NowExecShellMsg;
pub use win_ps::{NowExecWinPsFlags, NowExecWinPsMsg};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NowExecMessage {
    Capset(NowExecCapsetMsg),
    Abort(NowExecAbortMsg),
    CancelReq(NowExecCancelReqMsg),
    CancelRsp(NowExecCancelRspMsg),
    Result(NowExecResultMsg),
    Data(NowExecDataMsg),
    Run(NowExecRunMsg),
    // TODO: Define `Cmd` message in specification
    Process(NowExecProcessMsg),
    Shell(NowExecShellMsg),
    Batch(NowExecBatchMsg),
    WinPs(NowExecWinPsMsg),
    Pwsh(NowExecPwshMsg),
}

impl NowExecMessage {
    const NAME: &'static str = "NOW_EXEC_MSG";

    pub fn decode_from_body(header: NowHeader, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        match NowExecMsgKind(header.kind) {
            NowExecMsgKind::CAPSET => Ok(Self::Capset(NowExecCapsetMsg::new(
                NowExecCapsetFlags::from_bits_retain(header.flags),
            ))),
            NowExecMsgKind::ABORT => Ok(Self::Abort(NowExecAbortMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::CANCEL_REQ => Ok(Self::CancelReq(NowExecCancelReqMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::CANCEL_RSP => Ok(Self::CancelRsp(NowExecCancelRspMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::RESULT => Ok(Self::Result(NowExecResultMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::DATA => Ok(Self::Data(NowExecDataMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::RUN => Ok(Self::Run(NowExecRunMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::PROCESS => Ok(Self::Process(NowExecProcessMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::SHELL => Ok(Self::Shell(NowExecShellMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::BATCH => Ok(Self::Batch(NowExecBatchMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::WINPS => Ok(Self::WinPs(NowExecWinPsMsg::decode_from_body(header, src)?)),
            NowExecMsgKind::PWSH => Ok(Self::Pwsh(NowExecPwshMsg::decode_from_body(header, src)?)),
            _ => Err(invalid_field_err!("type", "invalid message type")),
        }
    }
}

impl Encode for NowExecMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        match self {
            Self::Capset(msg) => msg.encode(dst),
            Self::Abort(msg) => msg.encode(dst),
            Self::CancelReq(msg) => msg.encode(dst),
            Self::CancelRsp(msg) => msg.encode(dst),
            Self::Result(msg) => msg.encode(dst),
            Self::Data(msg) => msg.encode(dst),
            Self::Run(msg) => msg.encode(dst),
            Self::Process(msg) => msg.encode(dst),
            Self::Shell(msg) => msg.encode(dst),
            Self::Batch(msg) => msg.encode(dst),
            Self::WinPs(msg) => msg.encode(dst),
            Self::Pwsh(msg) => msg.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::Capset(msg) => msg.size(),
            Self::Abort(msg) => msg.size(),
            Self::CancelReq(msg) => msg.size(),
            Self::CancelRsp(msg) => msg.size(),
            Self::Result(msg) => msg.size(),
            Self::Data(msg) => msg.size(),
            Self::Run(msg) => msg.size(),
            Self::Process(msg) => msg.size(),
            Self::Shell(msg) => msg.size(),
            Self::Batch(msg) => msg.size(),
            Self::WinPs(msg) => msg.size(),
            Self::Pwsh(msg) => msg.size(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NowExecMsgKind(pub u8);

impl NowExecMsgKind {
    /// NOW-PROTO: NOW_EXEC_CAPSET_MSG_ID
    pub const CAPSET: Self = Self(0x00);
    /// NOW-PROTO: NOW_EXEC_ABORT_MSG_ID
    pub const ABORT: Self = Self(0x01);
    /// NOW-PROTO: NOW_EXEC_CANCEL_REQ_MSG_ID
    pub const CANCEL_REQ: Self = Self(0x02);
    /// NOW-PROTO: NOW_EXEC_CANCEL_RSP_MSG_ID
    pub const CANCEL_RSP: Self = Self(0x03);
    /// NOW-PROTO: NOW_EXEC_RESULT_MSG_ID
    pub const RESULT: Self = Self(0x04);
    /// NOW-PROTO: NOW_EXEC_DATA_MSG_ID
    pub const DATA: Self = Self(0x05);
    /// NOW-PROTO: NOW_EXEC_RUN_MSG_ID
    pub const RUN: Self = Self(0x10);
    // /// NOW-PROTO: NOW_EXEC_CMD_MSG_ID
    // pub const CMD: Self = Self(0x11);
    /// NOW-PROTO: NOW_EXEC_PROCESS_MSG_ID
    pub const PROCESS: Self = Self(0x12);
    /// NOW-PROTO: NOW_EXEC_SHELL_MSG_ID
    pub const SHELL: Self = Self(0x13);
    /// NOW-PROTO: NOW_EXEC_BATCH_MSG_ID
    pub const BATCH: Self = Self(0x14);
    /// NOW-PROTO: NOW_EXEC_WINPS_MSG_ID
    pub const WINPS: Self = Self(0x15);
    /// NOW-PROTO: NOW_EXEC_PWSH_MSG_ID
    pub const PWSH: Self = Self(0x16);
}
