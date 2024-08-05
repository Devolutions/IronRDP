use now_proto_pdu::*;
use rstest::rstest;

#[rstest]
#[case(0x00, &[0x00])]
#[case(0x7F, &[0x7F])]
#[case(0x80, &[0x80, 0x80])]
fn var_u16_roundtrip(#[case] value: u16, #[case] expected_encoded: &'static [u8]) {
    let mut encoded_value = [0u8; 4];
    let encoded_size = ironrdp_pdu::encode(&VarU16::new(value).unwrap(), &mut encoded_value).unwrap();

    assert_eq!(encoded_size, expected_encoded.len());
    assert_eq!(&encoded_value[..encoded_size], expected_encoded);

    let decoded_value = ironrdp_pdu::decode::<VarU16>(&encoded_value).unwrap();
    assert_eq!(decoded_value.value(), value);
}

#[rstest]
#[case(0x00, &[0x00])]
#[case(0x3F, &[0x3F])]
#[case(0x40, &[0x80, 0x40])]
#[case(-0x3F, &[0x7F])]
#[case(-0x40, &[0xC0, 0x40])]
fn var_i16_roundtrip(#[case] value: i16, #[case] expected_encoded: &'static [u8]) {
    let mut encoded_value = [0u8; 4];
    let encoded_size = ironrdp_pdu::encode(&VarI16::new(value).unwrap(), &mut encoded_value).unwrap();

    assert_eq!(encoded_size, expected_encoded.len());
    assert_eq!(&encoded_value[..encoded_size], expected_encoded);

    let decoded_value = ironrdp_pdu::decode::<VarI16>(&encoded_value).unwrap();
    assert_eq!(decoded_value.value(), value);
}

#[rstest]
#[case(0x00, &[0x00])]
#[case(0x3F, &[0x3F])]
#[case(0x40, &[0x40, 0x40])]
#[case(0x14000, &[0x81, 0x40, 0x00])]
#[case(0x3FFFFFFF, &[0xFF, 0xFF, 0xFF, 0xFF])]
fn var_u32_roundtrip(#[case] value: u32, #[case] expected_encoded: &'static [u8]) {
    let mut encoded_value = [0u8; 4];
    let encoded_size = ironrdp_pdu::encode(&VarU32::new(value).unwrap(), &mut encoded_value).unwrap();

    assert_eq!(encoded_size, expected_encoded.len());
    assert_eq!(&encoded_value[..encoded_size], expected_encoded);

    let decoded_value = ironrdp_pdu::decode::<VarU32>(&encoded_value).unwrap();
    assert_eq!(decoded_value.value(), value);
}

#[rstest]
#[case(0x00, &[0x00])]
#[case(0x1F, &[0x1F])]
#[case(0x20, &[0x40, 0x20])]
#[case(0x14000, &[0x81, 0x40, 0x00])]
#[case(0x1FFFFFFF, &[0xDF, 0xFF, 0xFF, 0xFF])]
#[case(-0x1F, &[0x3F])]
#[case(-0x1FFFFFFF, &[0xFF, 0xFF, 0xFF, 0xFF])]
fn var_i32_roundtrip(#[case] value: i32, #[case] expected_encoded: &'static [u8]) {
    let mut encoded_value = [0u8; 4];
    let encoded_size = ironrdp_pdu::encode(&VarI32::new(value).unwrap(), &mut encoded_value).unwrap();

    assert_eq!(encoded_size, expected_encoded.len());
    assert_eq!(&encoded_value[..encoded_size], expected_encoded);

    let decoded_value = ironrdp_pdu::decode::<VarI32>(&encoded_value).unwrap();
    assert_eq!(decoded_value.value(), value);
}

#[rstest]
#[case(0x00, &[0x00])]
#[case(0x1F, &[0x1F])]
#[case(0x20, &[0x20, 0x20])]
#[case(0x14000, &[0x41, 0x40, 0x00])]
#[case(0x1FFFFFFF, &[0x7F, 0xFF, 0xFF, 0xFF])]
#[case(0x1FFFFFFFFF, &[0x9F, 0xFF, 0xFF, 0xFF, 0xFF])]
#[case(0x1FFFFFFFFFFF, &[0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])]
#[case(0x1FFFFFFFFFFFFF, &[0xDF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])]
#[case(0x1FFFFFFFFFFFFFFF, &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])]
fn var_u64_roundtrip(#[case] value: u64, #[case] expected_encoded: &'static [u8]) {
    let mut encoded_value = [0u8; 8];
    let encoded_size = ironrdp_pdu::encode(&VarU64::new(value).unwrap(), &mut encoded_value).unwrap();

    assert_eq!(encoded_size, expected_encoded.len());
    assert_eq!(&encoded_value[..encoded_size], expected_encoded);

    let decoded_value = ironrdp_pdu::decode::<VarU64>(&encoded_value).unwrap();
    assert_eq!(decoded_value.value(), value);
}

#[rstest]
#[case(0x00, &[0x00])]
#[case(0x0F, &[0x0F])]
#[case(0x10, &[0x20, 0x10])]
#[case(0x021400, &[0x42, 0x14, 0x00])]
#[case(0x0FFFFFFF, &[0x6F, 0xFF, 0xFF, 0xFF])]
#[case(0x0FFFFFFFFF, &[0x8F, 0xFF, 0xFF, 0xFF, 0xFF])]
#[case(0x0FFFFFFFFFFF, &[0xAF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])]
#[case(0x0FFFFFFFFFFFFF, &[0xCF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])]
#[case(0x0FFFFFFFFFFFFFFF, &[0xEF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])]
#[case(-0x0F, &[0x1F])]
#[case(-0x0FFFFFFF, &[0x7F, 0xFF, 0xFF, 0xFF])]
fn var_i64_roundtrip(#[case] value: i64, #[case] expected_encoded: &'static [u8]) {
    let mut encoded_value = [0u8; 8];
    let encoded_size = ironrdp_pdu::encode(&VarI64::new(value).unwrap(), &mut encoded_value).unwrap();

    assert_eq!(encoded_size, expected_encoded.len());
    assert_eq!(&encoded_value[..encoded_size], expected_encoded);

    let decoded_value = ironrdp_pdu::decode::<VarI64>(&encoded_value).unwrap();
    assert_eq!(decoded_value.value(), value);
}

#[test]
fn constructed_var_int_too_large() {
    VarU16::new(0x8000).unwrap_err();
    VarI16::new(0x4000).unwrap_err();
    VarI16::new(-0x4000).unwrap_err();
    VarU32::new(0x40000000).unwrap_err();
    VarI32::new(0x20000000).unwrap_err();
    VarI32::new(-0x20000000).unwrap_err();
    VarU64::new(0x2000000000000000).unwrap_err();
    VarI64::new(0x1000000000000000).unwrap_err();
    VarI64::new(-0x1000000000000000).unwrap_err();
}
