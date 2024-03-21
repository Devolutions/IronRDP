use super::*;

const CHANNEL_ID: u32 = 0x0000_0003;
const REQ_ENCODED: [u8; 10] = [0x10, 0x03, 0x74, 0x65, 0x73, 0x74, 0x64, 0x76, 0x63, 0x00];
const RESP_ENCODED: [u8; 6] = [0x10, 0x03, 0x00, 0x00, 0x00, 0x00];

static REQ_DECODED_SERVER: OnceLock<DrdynvcServerPdu> = OnceLock::new();
static RESP_DECODED_CLIENT: OnceLock<DrdynvcClientPdu> = OnceLock::new();

fn req_decoded_server() -> &'static DrdynvcServerPdu {
    REQ_DECODED_SERVER
        .get_or_init(|| DrdynvcServerPdu::Create(CreateRequestPdu::new(CHANNEL_ID, String::from("testdvc"))))
}

fn resp_decoded_client() -> &'static DrdynvcClientPdu {
    RESP_DECODED_CLIENT.get_or_init(|| DrdynvcClientPdu::Create(CreateResponsePdu::new(CHANNEL_ID, CreationStatus::OK)))
}

#[test]
fn decodes_create_request() {
    test_decodes(&REQ_ENCODED, req_decoded_server());
}

#[test]
fn encodes_create_request() {
    test_encodes(req_decoded_server(), &REQ_ENCODED);
}

#[test]
fn decodes_create_response() {
    test_decodes(&RESP_ENCODED, resp_decoded_client());
}

#[test]
fn encodes_create_response() {
    test_encodes(resp_decoded_client(), &RESP_ENCODED);
}
