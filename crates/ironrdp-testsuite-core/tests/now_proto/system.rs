use expect_test::expect;

use ironrdp_testsuite_core::now_proto::now_msg_roundtrip;
use now_proto_pdu::*;

#[test]
fn roundtip_system_shutdown() {
    now_msg_roundtrip(
        NowSystemShutdownMsg {
            flags: NowSystemShutdownFlags::FORCE,
            message: NowVarStr::new("hello".to_string()).unwrap(),
            timeout: 0x12345678,
        },
        expect!["[0B, 00, 00, 00, 11, 03, 01, 00, 78, 56, 34, 12, 05, 68, 65, 6C, 6C, 6F, 00]"],
    );
}
