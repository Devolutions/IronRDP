use super::*;

const CHANNEL_ID: u32 = 0x03;
const PREFIX: [u8; 2] = [0x30, 0x03];
const DATA: [u8; 12] = [0x71; 12];

static ENCODED: OnceLock<Vec<u8>> = OnceLock::new();
static DECODED_CLIENT: OnceLock<DrdynvcClientPdu> = OnceLock::new();
static DECODED_SERVER: OnceLock<DrdynvcServerPdu> = OnceLock::new();

fn encoded() -> &'static Vec<u8> {
    ENCODED.get_or_init(|| {
        let mut result = PREFIX.to_vec();
        result.extend(&DATA);
        result
    })
}

fn decoded_client() -> &'static DrdynvcClientPdu {
    DECODED_CLIENT.get_or_init(|| DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, DATA.to_vec()))))
}

fn decoded_server() -> &'static DrdynvcServerPdu {
    DECODED_SERVER.get_or_init(|| DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(CHANNEL_ID, DATA.to_vec()))))
}

#[test]
fn decodes_data() {
    test_decodes(encoded(), decoded_client());
    test_decodes(encoded(), decoded_server());
}

#[test]
fn encodes_data() {
    test_encodes(decoded_client(), encoded());
    test_encodes(decoded_server(), encoded());
}
