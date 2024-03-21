use super::*;
use alloc::vec;
use lazy_static::lazy_static;

const CHANNEL_ID: u32 = 0x03;
const PREFIX: [u8; 2] = [0x30, 0x03];
const DATA: [u8; 12] = [0x71; 12];

lazy_static! {
    static ref ENCODED: Vec<u8> = {
        let mut result = PREFIX.to_vec();
        result.extend(DATA);
        result
    };
    static ref DECODED: DataPdu = DataPdu::new(CHANNEL_ID, DATA.to_vec());
}

#[test]
fn decodes_data() {
    let mut src = ReadCursor::new(&ENCODED);
    match DrdynvcClientPdu::decode(&mut src).unwrap() {
        DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(d)) => assert_eq!(*DECODED, d),
        _ => panic!("Expected Data"),
    }

    let mut src = ReadCursor::new(&ENCODED);
    match DrdynvcServerPdu::decode(&mut src).unwrap() {
        DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(d)) => assert_eq!(*DECODED, d),
        _ => panic!("Expected Data"),
    }
}

#[test]
fn encodes_data() {
    let data = &*DECODED;
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(ENCODED.as_slice(), buffer.as_slice());
}
