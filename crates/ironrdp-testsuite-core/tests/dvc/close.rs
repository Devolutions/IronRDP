use super::*;

const CHANNEL_ID: u32 = 0x0303;
const ENCODED: [u8; 3] = [0x41, 0x03, 0x03];

static DECODED_CLIENT: OnceLock<DrdynvcClientPdu> = OnceLock::new();
static DECODED_SERVER: OnceLock<DrdynvcServerPdu> = OnceLock::new();

fn decoded_client() -> &'static DrdynvcClientPdu {
    DECODED_CLIENT.get_or_init(|| DrdynvcClientPdu::Close(ClosePdu::new(CHANNEL_ID).with_cb_id_type(FieldType::U16)))
}

fn decoded_server() -> &'static DrdynvcServerPdu {
    DECODED_SERVER.get_or_init(|| DrdynvcServerPdu::Close(ClosePdu::new(CHANNEL_ID).with_cb_id_type(FieldType::U16)))
}

#[test]
fn decodes_close() {
    test_decodes(&ENCODED, decoded_client());
    test_decodes(&ENCODED, decoded_server());
}

#[test]
fn encodes_close() {
    test_encodes(decoded_client(), &ENCODED);
    test_encodes(decoded_server(), &ENCODED);
}
