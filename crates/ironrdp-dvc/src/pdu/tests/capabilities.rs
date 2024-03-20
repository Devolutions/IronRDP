use crate::vec;
use lazy_static::lazy_static;

use super::*;

const REQ_V1_ENCODED: [u8; 4] = [0x50, 0x00, 0x01, 0x00];
const REQ_V2_ENCODED: [u8; 12] = [0x50, 0x00, 0x02, 0x00, 0x33, 0x33, 0x11, 0x11, 0x3d, 0x0a, 0xa7, 0x04];
const RESP_V1_ENCODED: [u8; 4] = [0x50, 0x00, 0x01, 0x00];

lazy_static! {
    static ref REQ_V1_DECODED: CapabilitiesRequestPdu = CapabilitiesRequestPdu::new(CapsVersion::V1, None);
    static ref REQ_V2_DECODED: CapabilitiesRequestPdu =
        CapabilitiesRequestPdu::new(CapsVersion::V2, Some([0x3333, 0x1111, 0x0a3d, 0x04a7]));
    static ref RESP_V1_DECODED: CapabilitiesResponsePdu = CapabilitiesResponsePdu::new(CapsVersion::V1);
}

#[test]
fn decodes_request_v1() {
    let mut src = ReadCursor::new(&REQ_V1_ENCODED);
    match DrdynvcServerPdu::decode(&mut src).unwrap() {
        DrdynvcServerPdu::Capabilities(pdu) => assert_eq!(*REQ_V1_DECODED, pdu),
        _ => panic!("Expected DataFirst"),
    }
}

#[test]
fn encodes_request_v1() {
    let data = &*REQ_V1_DECODED;
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(REQ_V1_ENCODED.as_ref(), buffer.as_slice());
}

#[test]
fn decodes_request_v2() {
    let mut src = ReadCursor::new(&REQ_V2_ENCODED);
    match DrdynvcServerPdu::decode(&mut src).unwrap() {
        DrdynvcServerPdu::Capabilities(pdu) => assert_eq!(*REQ_V2_DECODED, pdu),
        _ => panic!("Expected DataFirst"),
    }
}

#[test]
fn encodes_request_v2() {
    let data = &*REQ_V2_DECODED;
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(REQ_V2_ENCODED.as_ref(), buffer.as_slice());
}

#[test]
fn decodes_response_v1() {
    let mut src = ReadCursor::new(&RESP_V1_ENCODED);
    match DrdynvcClientPdu::decode(&mut src).unwrap() {
        DrdynvcClientPdu::Capabilities(pdu) => assert_eq!(*RESP_V1_DECODED, pdu),
        _ => panic!("Expected DataFirst"),
    }
}

#[test]
fn encodes_response_v1() {
    let data = &*RESP_V1_DECODED;
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(RESP_V1_ENCODED.as_ref(), buffer.as_slice());
}
