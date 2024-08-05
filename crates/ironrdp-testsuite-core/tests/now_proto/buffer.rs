use now_proto_pdu::*;

use rstest::rstest;

#[rstest]
#[case(b"hello", &[0x05, 0x00, 0x00, 0x00, b'h', b'e', b'l', b'l', b'o'])]
#[case(&[], &[0x00, 0x00, 0x00, 0x00])]
fn now_lrgbuf_roundtrip(#[case] value: &[u8], #[case] expected_encoded: &[u8]) {
    let mut encoded_value = [0u8; 32];
    let encoded_size = ironrdp_pdu::encode(&NowLrgBuf::new(value).unwrap(), &mut encoded_value).unwrap();

    assert_eq!(encoded_size, expected_encoded.len());
    assert_eq!(&encoded_value[..encoded_size], expected_encoded);

    let decoded_value = ironrdp_pdu::decode::<NowLrgBuf>(&encoded_value).unwrap();
    assert_eq!(decoded_value.value(), value);
}

#[rstest]
#[case(b"hello", &[0x05, b'h', b'e', b'l', b'l', b'o'])]
#[case(&[], &[0x00])]
fn now_varbuf_roundtrip(#[case] value: &[u8], #[case] expected_encoded: &[u8]) {
    let mut encoded_value = [0u8; 32];
    let encoded_size = ironrdp_pdu::encode(&NowVarBuf::new(value).unwrap(), &mut encoded_value).unwrap();

    assert_eq!(encoded_size, expected_encoded.len());
    assert_eq!(&encoded_value[..encoded_size], expected_encoded);

    let decoded_value = ironrdp_pdu::decode::<NowVarBuf>(&encoded_value).unwrap();
    assert_eq!(decoded_value.value(), value);
}
