use super::*;

const CHANNEL_ID: u32 = 0x03;
const PREFIX: [u8; 2] = [0x30, 0x03];
const DATA: [u8; 12] = [0x71; 12];

lazy_static! {
    static ref ENCODED: Vec<u8> = {
        let mut result = PREFIX.to_vec();
        result.extend(DATA);
        result
    };
    static ref DECODED_CLIENT: DrdynvcClientPdu =
        DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, DATA.to_vec())));
    static ref DECODED_SERVER: DrdynvcServerPdu =
        DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, DATA.to_vec())));
}

#[test]
fn decodes_data() {
    test_decodes(&ENCODED, &*DECODED_CLIENT);
    test_decodes(&ENCODED, &*DECODED_SERVER);
}

#[test]
fn encodes_data() {
    test_encodes(&*DECODED_CLIENT, &ENCODED);
    test_encodes(&*DECODED_SERVER, &ENCODED);
}
