use crate::vec;
use lazy_static::lazy_static;

use super::*;

const CHANNEL_ID: u32 = 0x0303;
const ENCODED: [u8; 3] = [0x41, 0x03, 0x03];

lazy_static! {
    static ref DECODED: ClosePdu = {
        let mut pdu = ClosePdu::new(CHANNEL_ID);
        pdu.header.cb_id = FieldType::U16;
        pdu
    };
}

#[test]
fn decodes_close() {
    let mut src = ReadCursor::new(&ENCODED);
    match DrdynvcClientPdu::decode(&mut src).unwrap() {
        DrdynvcClientPdu::Close(pdu) => assert_eq!(*DECODED, pdu),
        _ => panic!("Expected Close"),
    }

    let mut src = ReadCursor::new(&ENCODED);
    match DrdynvcServerPdu::decode(&mut src).unwrap() {
        DrdynvcServerPdu::Close(pdu) => assert_eq!(*DECODED, pdu),
        _ => panic!("Expected Close"),
    }
}

#[test]
fn encodes_close() {
    let data = &*DECODED;
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(ENCODED.as_slice(), buffer.as_slice());
}
