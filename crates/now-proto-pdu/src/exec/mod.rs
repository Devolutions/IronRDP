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

use crate::{NowHeader, PduEncode, PduResult, ReadCursor, WriteCursor};

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

    pub fn decode_from_body(header: NowHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
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
            _ => Err(invalid_message_err!("type", "invalid message type")),
        }
    }
}

impl PduEncode for NowExecMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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
pub(crate) struct NowExecMsgKind(pub u8);

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

#[cfg(all(test, feature = "std"))]
mod tests {
    use alloc::string::ToString;

    use super::*;
    use crate::{test_utils::now_msg_roundtrip, NowSeverity, NowStatus, NowStatusCode, NowVarBuf, NowVarStr};

    use expect_test::expect;

    #[test]
    fn roundtrip_exec_capset() {
        now_msg_roundtrip(
            NowExecCapsetMsg::new(NowExecCapsetFlags::all()),
            expect!["[00, 00, 00, 00, 13, 00, FF, 00]"],
        );
    }

    #[test]
    fn roundtrip_exec_abort() {
        now_msg_roundtrip(
            NowExecAbortMsg::new(0x12345678, NowStatus::new(NowSeverity::Fatal, NowStatusCode::FAILURE)),
            expect!["[08, 00, 00, 00, 13, 01, 00, 00, 78, 56, 34, 12, C0, 00, FF, FF]"],
        );
    }

    #[test]
    fn roundtrip_exec_cancel_req() {
        now_msg_roundtrip(
            NowExecCancelReqMsg::new(0x12345678),
            expect!["[04, 00, 00, 00, 13, 02, 00, 00, 78, 56, 34, 12]"],
        );
    }

    #[test]
    fn roundtrip_exec_cancel_rsp() {
        now_msg_roundtrip(
            NowExecCancelRspMsg::new(0x12345678, NowStatus::new(NowSeverity::Error, NowStatusCode::FAILURE)),
            expect!["[08, 00, 00, 00, 13, 03, 00, 00, 78, 56, 34, 12, 80, 00, FF, FF]"],
        );
    }

    #[test]
    fn roundtrip_exec_result() {
        now_msg_roundtrip(
            NowExecResultMsg::new(0x12345678, NowStatus::new(NowSeverity::Error, NowStatusCode::FAILURE)),
            expect!["[08, 00, 00, 00, 13, 04, 00, 00, 78, 56, 34, 12, 80, 00, FF, FF]"],
        );
    }

    #[test]
    fn roundtrip_exec_data() {
        now_msg_roundtrip(
            NowExecDataMsg::new(
                NowExecDataFlags::LAST,
                0x12345678,
                NowVarBuf::new(vec![0x01, 0x02, 0x03]).unwrap(),
            ),
            expect!["[08, 00, 00, 00, 13, 05, 02, 00, 78, 56, 34, 12, 03, 01, 02, 03]"],
        );
    }

    #[test]
    fn roundtrip_exec_run() {
        now_msg_roundtrip(
            NowExecRunMsg::new(0x1234567, NowVarStr::new("hello".to_string()).unwrap()),
            expect!["[0B, 00, 00, 00, 13, 10, 00, 00, 67, 45, 23, 01, 05, 68, 65, 6C, 6C, 6F, 00]"],
        );
    }

    #[test]
    fn roundtrip_exec_process() {
        now_msg_roundtrip(
            NowExecProcessMsg::new(
                0x12345678,
                NowVarStr::new("a".to_string()).unwrap(),
                NowVarStr::new("b".to_string()).unwrap(),
                NowVarStr::new("c".to_string()).unwrap(),
            ),
            expect!["[0D, 00, 00, 00, 13, 12, 00, 00, 78, 56, 34, 12, 01, 61, 00, 01, 62, 00, 01, 63, 00]"],
        );
    }

    #[test]
    fn roundtrip_exec_shell() {
        now_msg_roundtrip(
            NowExecShellMsg::new(
                0x12345678,
                NowVarStr::new("a".to_string()).unwrap(),
                NowVarStr::new("b".to_string()).unwrap(),
            ),
            expect!["[0A, 00, 00, 00, 13, 13, 00, 00, 78, 56, 34, 12, 01, 61, 00, 01, 62, 00]"],
        );
    }

    #[test]
    fn roundtrip_exec_batch() {
        now_msg_roundtrip(
            NowExecBatchMsg::new(0x12345678, NowVarStr::new("a".to_string()).unwrap()),
            expect!["[07, 00, 00, 00, 13, 14, 00, 00, 78, 56, 34, 12, 01, 61, 00]"],
        );
    }

    #[test]
    fn roundtrip_exec_ps() {
        now_msg_roundtrip(
            NowExecWinPsMsg::new(0x12345678, NowVarStr::new("a".to_string()).unwrap())
                .with_flags(NowExecWinPsFlags::NO_PROFILE)
                .with_execution_policy(NowVarStr::new("b".to_string()).unwrap())
                .with_configuration_name(NowVarStr::new("c".to_string()).unwrap()),
            expect!["[0D, 00, 00, 00, 13, 15, D0, 00, 78, 56, 34, 12, 01, 61, 00, 01, 62, 00, 01, 63, 00]"],
        );
    }

    #[test]
    fn roundtrip_exec_pwsh() {
        now_msg_roundtrip(
            NowExecPwshMsg::new(0x12345678, NowVarStr::new("a".to_string()).unwrap())
                .with_flags(NowExecWinPsFlags::NO_PROFILE)
                .with_execution_policy(NowVarStr::new("b".to_string()).unwrap())
                .with_configuration_name(NowVarStr::new("c".to_string()).unwrap()),
            expect!["[0D, 00, 00, 00, 13, 16, D0, 00, 78, 56, 34, 12, 01, 61, 00, 01, 62, 00, 01, 63, 00]"],
        );
    }
}
