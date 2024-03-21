use super::*;

const REQ_V1_ENCODED: [u8; 4] = [0x50, 0x00, 0x01, 0x00];
const REQ_V2_ENCODED: [u8; 12] = [0x50, 0x00, 0x02, 0x00, 0x33, 0x33, 0x11, 0x11, 0x3d, 0x0a, 0xa7, 0x04];
const RESP_V1_ENCODED: [u8; 4] = [0x50, 0x00, 0x01, 0x00];

lazy_static! {
    static ref REQ_V1_DECODED_SERVER: DrdynvcServerPdu =
        DrdynvcServerPdu::Capabilities(CapabilitiesRequestPdu::new(CapsVersion::V1, None));
    static ref REQ_V2_DECODED_SERVER: DrdynvcServerPdu = DrdynvcServerPdu::Capabilities(CapabilitiesRequestPdu::new(
        CapsVersion::V2,
        Some([0x3333, 0x1111, 0x0a3d, 0x04a7])
    ));
    static ref RESP_V1_DECODED_CLIENT: DrdynvcClientPdu =
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V1));
}

#[test]
fn decodes_request_v1() {
    test_decodes(&REQ_V1_ENCODED, &*REQ_V1_DECODED_SERVER);
}

#[test]
fn encodes_request_v1() {
    test_encodes(&*REQ_V1_DECODED_SERVER, &REQ_V1_ENCODED);
}

#[test]
fn decodes_request_v2() {
    test_decodes(&REQ_V2_ENCODED, &*REQ_V2_DECODED_SERVER);
}

#[test]
fn encodes_request_v2() {
    test_encodes(&*REQ_V2_DECODED_SERVER, &REQ_V2_ENCODED);
}

#[test]
fn decodes_response_v1() {
    test_decodes(&RESP_V1_ENCODED, &*RESP_V1_DECODED_CLIENT);
}

#[test]
fn encodes_response_v1() {
    test_encodes(&*RESP_V1_DECODED_CLIENT, &RESP_V1_ENCODED);
}
