use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::{Decode as _, Encode as _};
use now_proto_pdu::{NowSeverity, NowStatus, NowStatusCode};

#[test]
fn now_status_roundtrip() {
    let status = NowStatus::new(NowSeverity::Error, NowStatusCode::FILE_NOT_FOUND)
        .with_kind(0x07)
        .unwrap();

    let mut buf = [0; 4];
    let mut cursor = WriteCursor::new(&mut buf);
    status.encode(&mut cursor).unwrap();

    assert_eq!(&buf, &[0x80, 0x07, 0x02, 0x00]);

    let mut cursor = ReadCursor::new(&buf);
    let decoded = NowStatus::decode(&mut cursor).unwrap();

    assert_eq!(status, decoded);
}
