use super::*;

// MS-RDPEDYC 4.3.3: a DYNVC_DATA_FIRST_COMPRESSED PDU on channel 3.
const DATA_FIRST_COMPRESSED_ENCODED: [u8; 12] =
    [0x64, 0x03, 0x7b, 0x0c, 0xe0, 0x26, 0x38, 0xc4, 0x3f, 0xf4, 0x74, 0x01];
const DATA_FIRST_COMPRESSED_LENGTH: u32 = 0x0c7b;

// MS-RDPEDYC 4.3.4: a DYNVC_DATA_COMPRESSED PDU on channel 3.
const DATA_COMPRESSED_ENCODED: [u8; 9] = [0x70, 0x03, 0xe0, 0x26, 0x88, 0x7f, 0xe8, 0xf4, 0x02];

const CHANNEL_ID: u32 = 0x03;

fn data_first_compressed() -> DrdynvcDataPdu {
    DrdynvcDataPdu::DataFirstCompressed(DataFirstCompressedPdu::new(
        CHANNEL_ID,
        DATA_FIRST_COMPRESSED_LENGTH,
        DATA_FIRST_COMPRESSED_ENCODED[4..].to_vec(),
    ))
}

fn data_compressed() -> DrdynvcDataPdu {
    DrdynvcDataPdu::DataCompressed(DataCompressedPdu::new(
        CHANNEL_ID,
        DATA_COMPRESSED_ENCODED[2..].to_vec(),
    ))
}

#[test]
fn decodes_data_first_compressed() {
    test_decodes(
        &DATA_FIRST_COMPRESSED_ENCODED,
        &DrdynvcServerPdu::Data(data_first_compressed()),
    );
    test_decodes(
        &DATA_FIRST_COMPRESSED_ENCODED,
        &DrdynvcClientPdu::Data(data_first_compressed()),
    );
}

#[test]
fn encodes_data_first_compressed() {
    test_encodes(
        &DrdynvcServerPdu::Data(data_first_compressed()),
        &DATA_FIRST_COMPRESSED_ENCODED,
    );
}

#[test]
fn decodes_data_compressed() {
    test_decodes(&DATA_COMPRESSED_ENCODED, &DrdynvcServerPdu::Data(data_compressed()));
    test_decodes(&DATA_COMPRESSED_ENCODED, &DrdynvcClientPdu::Data(data_compressed()));
}

#[test]
fn encodes_data_compressed() {
    test_encodes(&DrdynvcServerPdu::Data(data_compressed()), &DATA_COMPRESSED_ENCODED);
}
