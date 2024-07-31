use expect_test::expect;

use ironrdp_testsuite_core::now_proto::now_msg_roundtrip;
use now_proto_pdu::*;

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
