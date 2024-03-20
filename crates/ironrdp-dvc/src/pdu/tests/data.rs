use super::*;
use crate::vec;
use lazy_static::lazy_static;

const DATA_CHANNEL_ID: u32 = 0x03;
const DATA_PREFIX: [u8; 2] = [0x30, 0x03];
const DATA_DATA: [u8; 12] = [0x71; 12];

lazy_static! {
    static ref DATA_ENCODED: Vec<u8> = {
        let mut result = DATA_PREFIX.to_vec();
        result.extend(DATA_DATA);
        result
    };
    static ref DATA_DECODED: DataPdu = DataPdu::new(DATA_CHANNEL_ID, DATA_DATA.to_vec());
}

#[test]
fn decodes_data_pdu() {
    let mut src = ReadCursor::new(&DATA_ENCODED);
    match DrdynvcClientPdu::decode(&mut src).unwrap() {
        DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(d)) => assert_eq!(*DATA_DECODED, d),
        _ => panic!("Expected DataFirst"),
    }

    let mut src = ReadCursor::new(&DATA_ENCODED);
    match DrdynvcServerPdu::decode(&mut src).unwrap() {
        DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(d)) => assert_eq!(*DATA_DECODED, d),
        _ => panic!("Expected DataFirst"),
    }
}

#[test]
fn encodes_data_pdu() {
    let data = &*DATA_DECODED;
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(DATA_ENCODED.as_slice(), buffer.as_slice());
}
