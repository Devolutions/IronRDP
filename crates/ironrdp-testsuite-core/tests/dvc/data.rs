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

// oracle: spec MS-RDPEDYC §2.2.3.2 and §3.1.5.1.2.
#[test]
fn an_empty_message_emits_one_header_only_data_pdu() {
    let messages = ironrdp_dvc::encode_dvc_messages(
        7,
        vec![Box::new(TestPayload(Vec::new()))],
        ironrdp_svc::ChannelFlags::SHOW_PROTOCOL,
    )
    .unwrap();
    assert_eq!(messages.len(), 1, "an empty DVC message must not be discarded");
    assert_eq!(
        messages[0].encode_unframed_pdu().unwrap(),
        [
            0x30, // DATA command, no reserved flags, one-byte channel ID.
            0x07, // ChannelId.
        ]
    );
}

// oracle: spec MS-RDPEDYC §2.2.3.2.
#[test]
fn the_largest_unfragmented_message_uses_a_data_pdu() {
    let payload = vec![0x42; 1590];
    let messages = ironrdp_dvc::encode_dvc_messages(
        7,
        vec![Box::new(TestPayload(payload.clone()))],
        ironrdp_svc::ChannelFlags::empty(),
    )
    .unwrap();
    assert_eq!(messages.len(), 1);
    let wire = messages[0].encode_unframed_pdu().unwrap();
    let decoded = ironrdp_core::decode::<DrdynvcServerPdu>(&wire).unwrap();
    let DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(data)) = decoded else {
        panic!("a 1590-byte message must use one DATA PDU, not DATA_FIRST");
    };
    assert_eq!(data.data(), payload);
}
