use super::*;

const CHANNEL_ID: u32 = 0x0303;
const ENCODED: [u8; 3] = [0x41, 0x03, 0x03];

lazy_static! {
    static ref DECODED_CLIENT: DrdynvcClientPdu =
        DrdynvcClientPdu::Close(ClosePdu::new(CHANNEL_ID).with_cb_id_type(FieldType::U16));
    static ref DECODED_SERVER: DrdynvcServerPdu =
        DrdynvcServerPdu::Close(ClosePdu::new(CHANNEL_ID).with_cb_id_type(FieldType::U16));
}

#[test]
fn decodes_close() {
    test_decodes(&ENCODED, &*DECODED_CLIENT);
    test_decodes(&ENCODED, &*DECODED_SERVER);
}

#[test]
fn encodes_close() {
    test_encodes(&*DECODED_CLIENT, &ENCODED);
    test_encodes(&*DECODED_SERVER, &ENCODED);
}
