use expect_test::expect;

use ironrdp_testsuite_core::now_proto::now_msg_roundtrip;
use now_proto_pdu::*;

#[test]
fn roundtrip_session_lock() {
    now_msg_roundtrip(
        NowSessionLockMsg::default(),
        expect!["[00, 00, 00, 00, 12, 01, 00, 00]"],
    );
}

#[test]
fn roundtrip_session_logoff() {
    now_msg_roundtrip(
        NowSessionLogoffMsg::default(),
        expect!["[00, 00, 00, 00, 12, 02, 00, 00]"],
    );
}

#[test]
fn roundtip_session_msgbox_req() {
    now_msg_roundtrip(
        NowSessionMsgBoxReqMsg::new(
            0x76543210,
            NowVarStr::new("hello".to_string()).unwrap(),
        ).unwrap().with_response().with_style(NowMessageBoxStyle::ABORT_RETRY_IGNORE)
        .with_title(NowVarStr::new("world".to_string()).unwrap())
        .unwrap()
        .with_timeout(3),
        expect!["[1A, 00, 00, 00, 12, 03, 0F, 00, 10, 32, 54, 76, 02, 00, 00, 00, 03, 00, 00, 00, 05, 77, 6F, 72, 6C, 64, 00, 05, 68, 65, 6C, 6C, 6F, 00]"]
    );
}

#[test]
fn roundtrip_session_msgbox_rsp() {
    now_msg_roundtrip(
        NowSessionMsgBoxRspMsg::new(0x01234567, NowMsgBoxResponse::RETRY),
        expect!["[08, 00, 00, 00, 12, 04, 00, 00, 67, 45, 23, 01, 04, 00, 00, 00]"],
    );
}
