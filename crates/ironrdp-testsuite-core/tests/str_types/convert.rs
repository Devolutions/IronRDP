use ironrdp_str::{utf16_units_to_le_bytes, utf16le_bytes_to_units};
use rstest::rstest;

fn make_utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

#[rstest]
#[case("")]
#[case("hi")]
#[case("\u{1F600}")]
fn bytes_to_units(#[case] s: &str) {
    let expected: Vec<u16> = s.encode_utf16().collect();
    assert_eq!(utf16le_bytes_to_units(&make_utf16le(s)).unwrap(), expected);
}

#[rstest]
#[case("")]
#[case("hi")]
#[case("\u{1F600}")]
fn units_to_bytes(#[case] s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    assert_eq!(utf16_units_to_le_bytes(&units).as_ref(), make_utf16le(s).as_slice());
}

#[test]
fn bytes_to_units_odd_length_returns_none() {
    assert!(utf16le_bytes_to_units(&[0x41]).is_none());
}

// Property: bytes → units → bytes is a lossless round-trip for any valid UTF-8 string.
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]
    #[test]
    fn round_trip_prop(s in "\\PC{0,20}") {
        let bytes = make_utf16le(&s);
        let units = utf16le_bytes_to_units(&bytes).unwrap();
        let recovered = utf16_units_to_le_bytes(&units);
        proptest::prop_assert_eq!(recovered.as_ref(), bytes.as_slice());
    }
}
