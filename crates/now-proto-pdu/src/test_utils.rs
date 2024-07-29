//! Various test utilities
use alloc::vec::Vec;
use expect_test::Expect;

use ironrdp_pdu::cursor::ReadCursor;
use ironrdp_pdu::PduDecode as _;

use crate::NowMessage;

pub(crate) fn now_msg_roundtrip(msg: impl Into<NowMessage>, expected_bytes: Expect) {
    let msg = msg.into();

    let mut buf = Vec::new();
    let _ = ironrdp_pdu::encode_buf(&msg, &mut buf).unwrap();

    expected_bytes.assert_eq(&format!("{:02X?}", buf));

    let mut cursor = ReadCursor::new(&buf);
    let decoded = NowMessage::decode(&mut cursor).unwrap();

    assert_eq!(msg, decoded);
}
