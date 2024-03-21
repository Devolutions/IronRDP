use super::*;

const CHANNEL_ID: u32 = 0x0000_0003;
const REQ_ENCODED: [u8; 10] = [0x10, 0x03, 0x74, 0x65, 0x73, 0x74, 0x64, 0x76, 0x63, 0x00];
const RESP_ENCODED: [u8; 6] = [0x10, 0x03, 0x00, 0x00, 0x00, 0x00];

lazy_static! {
    static ref REQ_DECODED_SERVER: DrdynvcServerPdu =
        DrdynvcServerPdu::Create(CreateRequestPdu::new(CHANNEL_ID, String::from("testdvc")));
    static ref RESP_DECODED_CLIENT: DrdynvcClientPdu =
        DrdynvcClientPdu::Create(CreateResponsePdu::new(CHANNEL_ID, CreationStatus::OK));
}

#[test]
fn decodes_create_request() {
    test_decodes(&REQ_ENCODED, &*REQ_DECODED_SERVER);
}

#[test]
fn encodes_create_request() {
    test_encodes(&*REQ_DECODED_SERVER, &REQ_ENCODED);
}

#[test]
fn decodes_create_response() {
    test_decodes(&RESP_ENCODED, &*RESP_DECODED_CLIENT);
}

#[test]
fn encodes_create_response() {
    test_encodes(&*RESP_DECODED_CLIENT, &RESP_ENCODED);
}
