use super::*;

const REQ_V1_ENCODED: [u8; 4] = [0x50, 0x00, 0x01, 0x00];
const REQ_V2_ENCODED: [u8; 12] = [0x50, 0x00, 0x02, 0x00, 0x33, 0x33, 0x11, 0x11, 0x3d, 0x0a, 0xa7, 0x04];
const RESP_V1_ENCODED: [u8; 4] = [0x50, 0x00, 0x01, 0x00];

static REQ_V1_DECODED_SERVER: OnceLock<DrdynvcServerPdu> = OnceLock::new();
static REQ_V2_DECODED_SERVER: OnceLock<DrdynvcServerPdu> = OnceLock::new();
static RESP_V1_DECODED_CLIENT: OnceLock<DrdynvcClientPdu> = OnceLock::new();

fn req_v1_decoded_server() -> &'static DrdynvcServerPdu {
    REQ_V1_DECODED_SERVER
        .get_or_init(|| DrdynvcServerPdu::Capabilities(CapabilitiesRequestPdu::new(CapsVersion::V1, None)))
}

fn req_v2_decoded_server() -> &'static DrdynvcServerPdu {
    REQ_V2_DECODED_SERVER.get_or_init(|| {
        DrdynvcServerPdu::Capabilities(CapabilitiesRequestPdu::new(
            CapsVersion::V2,
            Some([0x3333, 0x1111, 0x0a3d, 0x04a7]),
        ))
    })
}

fn resp_v1_decoded_client() -> &'static DrdynvcClientPdu {
    RESP_V1_DECODED_CLIENT.get_or_init(|| DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V1)))
}

#[test]
fn decodes_request_v1() {
    test_decodes(&REQ_V1_ENCODED, req_v1_decoded_server());
}

#[test]
fn encodes_request_v1() {
    test_encodes(req_v1_decoded_server(), &REQ_V1_ENCODED);
}

#[test]
fn decodes_request_v2() {
    test_decodes(&REQ_V2_ENCODED, req_v2_decoded_server());
}

#[test]
fn encodes_request_v2() {
    test_encodes(req_v2_decoded_server(), &REQ_V2_ENCODED);
}

#[test]
fn decodes_response_v1() {
    test_decodes(&RESP_V1_ENCODED, resp_v1_decoded_client());
}

#[test]
fn encodes_response_v1() {
    test_encodes(resp_v1_decoded_client(), &RESP_V1_ENCODED);
}
