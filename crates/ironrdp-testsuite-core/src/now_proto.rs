//! Various test utilities
use expect_test::Expect;

use ironrdp_core::Decode as _;
use ironrdp_core::ReadCursor;

use now_proto_pdu::NowMessage;

pub fn now_msg_roundtrip(msg: impl Into<NowMessage>, expected_bytes: Expect) {
    let msg = msg.into();

    let buf = ironrdp_core::encode_vec(&msg).unwrap();

    expected_bytes.assert_eq(&format!("{:02X?}", buf));

    let mut cursor = ReadCursor::new(&buf);
    let decoded = NowMessage::decode(&mut cursor).unwrap();

    assert_eq!(msg, decoded);
}
