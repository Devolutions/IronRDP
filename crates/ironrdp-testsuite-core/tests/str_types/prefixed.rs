use expect_test::expect;
use ironrdp_core::{DecodeOwned as _, ReadCursor, encode_vec};
use ironrdp_str::prefixed::{
    CbPrefixedStringNoNull, CbPrefixedStringNullExcluded, CbPrefixedStringNullIncluded, Cch32PrefixedString,
    CchPrefixedString, LengthPrefix, NullTerminatorPolicy, UnicodeStringField,
};

fn encode_decode_roundtrip<P: LengthPrefix, N: NullTerminatorPolicy>(s: &str) -> String
where
    UnicodeStringField<P, N>: ironrdp_core::Encode + ironrdp_core::DecodeOwned,
{
    let field = UnicodeStringField::<P, N>::new(s.to_owned());
    let encoded = encode_vec(&field).unwrap();
    let decoded = UnicodeStringField::<P, N>::decode_owned(&mut ReadCursor::new(&encoded)).unwrap();
    decoded.to_native().unwrap().into_owned()
}

// ── Non-BMP correctness ───────────────────────────────────────────────────

#[test]
fn non_bmp_cch_null_counted() {
    // U+1F600 = 2 code units. cchPCB should be 3 (2 + null).
    let s = "\u{1F600}";
    let field = CchPrefixedString::new(s.to_owned());
    let encoded = encode_vec(&field).unwrap();
    // Prefix = 3 (u16 LE) + 2 code units * 2 bytes + null * 2 bytes = 2 + 4 + 2 = 8 bytes
    assert_eq!(encoded.len(), 8);
    let prefix = u16::from_le_bytes([encoded[0], encoded[1]]);
    assert_eq!(prefix, 3, "cch must include null; non-BMP counts as 2 code units");
    use ironrdp_str::prefixed::{CchU16, NullCounted};
    assert_eq!(encode_decode_roundtrip::<CchU16, NullCounted>(s), s);
}

#[test]
fn non_bmp_cb_null_excluded() {
    // U+1F600 = 2 code units = 4 bytes. cbDomain should be 4 (bytes, null excluded).
    let s = "\u{1F600}";
    let field = CbPrefixedStringNullExcluded::new(s.to_owned());
    let encoded = encode_vec(&field).unwrap();
    // Prefix = 4 (u16 LE) + 4 bytes content + null 2 bytes = 2 + 4 + 2 = 8 bytes
    assert_eq!(encoded.len(), 8);
    let prefix_bytes = u16::from_le_bytes([encoded[0], encoded[1]]);
    assert_eq!(prefix_bytes, 4, "cb must not include null bytes");
    use ironrdp_str::prefixed::{CbU16, NullUncounted};
    assert_eq!(encode_decode_roundtrip::<CbU16, NullUncounted>(s), s);
}

// ── Round-trips for all null policy variants ──────────────────────────────

#[test]
fn round_trip_cch_null_counted() {
    use ironrdp_str::prefixed::{CchU16, NullCounted};
    assert_eq!(encode_decode_roundtrip::<CchU16, NullCounted>("hello"), "hello");
}

#[test]
fn round_trip_cch32_null_counted() {
    use ironrdp_str::prefixed::{CchU32, NullCounted};
    assert_eq!(encode_decode_roundtrip::<CchU32, NullCounted>("hello"), "hello");
}

#[test]
fn round_trip_cb_null_excluded() {
    use ironrdp_str::prefixed::{CbU16, NullUncounted};
    assert_eq!(encode_decode_roundtrip::<CbU16, NullUncounted>("hello"), "hello");
}

#[test]
fn round_trip_cb_null_included() {
    use ironrdp_str::prefixed::{CbU16, NullCounted};
    assert_eq!(encode_decode_roundtrip::<CbU16, NullCounted>("hello"), "hello");
}

#[test]
fn round_trip_cb_no_null() {
    use ironrdp_str::prefixed::{CbU16, NoNull};
    assert_eq!(encode_decode_roundtrip::<CbU16, NoNull>("hello"), "hello");
}

// ── Empty string edge cases ───────────────────────────────────────────────

#[test]
fn empty_string_null_uncounted() {
    // cbDomain=0 with NullUncounted: prefix=0, then a null terminator on wire.
    let field = CbPrefixedStringNullExcluded::new(String::new());
    let encoded = encode_vec(&field).unwrap();
    assert_eq!(encoded.len(), 4); // 2-byte prefix (0) + 2-byte null
    let decoded = CbPrefixedStringNullExcluded::decode_owned(&mut ReadCursor::new(&encoded)).unwrap();
    assert_eq!(decoded.to_native().unwrap().as_ref(), "");
}

#[test]
fn empty_string_no_null() {
    let field = CbPrefixedStringNoNull::new(String::new());
    let encoded = encode_vec(&field).unwrap();
    assert_eq!(encoded.len(), 2); // 2-byte prefix (0), no null
    let decoded = CbPrefixedStringNoNull::decode_owned(&mut ReadCursor::new(&encoded)).unwrap();
    assert_eq!(decoded.to_native().unwrap().as_ref(), "");
}

// ── Rejection of odd byte counts ─────────────────────────────────────────

#[test]
fn rejects_odd_byte_count() {
    // cb = 3 (odd) is invalid for a UTF-16 string (structural, not UTF-16 validity).
    let wire: &[u8] = &[0x03, 0x00, 0x41, 0x00, 0x00, 0x00]; // cb=3, 'A', null
    let err = CbPrefixedStringNullExcluded::decode_owned(&mut ReadCursor::new(wire)).unwrap_err();
    expect![[r#"
        Error {
            context: "<ironrdp_str::prefixed::UnicodeStringField<_, _> as ironrdp_core::decode::DecodeOwned>::decode_owned",
            kind: InvalidField {
                field: "length prefix",
                reason: "odd byte count for utf-16 string field",
            },
            source: None,
        }
    "#]].assert_debug_eq(&err);
}

// ── Lone surrogates: decode succeeds, to_str() fails ─────────────────────

#[test]
fn lone_surrogate_decode_succeeds_to_str_fails() {
    // cb=2, lone high surrogate D800. Decode no longer validates; to_str() reports error.
    let wire: &[u8] = &[0x02, 0x00, 0x00, 0xD8]; // cb=2, code unit 0xD800
    let decoded = CbPrefixedStringNoNull::decode_owned(&mut ReadCursor::new(wire)).unwrap();
    let err = decoded.to_native().unwrap_err();
    expect![[r#"
        InvalidUtf16
    "#]]
    .assert_debug_eq(&err);
    assert!(decoded.to_native_lossy().contains('\u{FFFD}'));
}

// ── from_wire_units / to_wire_units / into_wire_units ────────────────────────

#[test]
fn from_wire_units_round_trip() {
    use ironrdp_str::prefixed::{CbU16, NullUncounted};
    let units: Vec<u16> = "hello".encode_utf16().collect();
    let field = UnicodeStringField::<CbU16, NullUncounted>::from_wire_units(units.clone());
    assert_eq!(field.to_native().unwrap().as_ref(), "hello");
    assert_eq!(field.to_wire_units().as_ref(), units.as_slice());
}

#[test]
fn into_wire_units_from_decode() {
    let field = CbPrefixedStringNoNull::new("abc".to_owned());
    let encoded = encode_vec(&field).unwrap();
    let decoded = CbPrefixedStringNoNull::decode_owned(&mut ReadCursor::new(&encoded)).unwrap();
    let units = decoded.into_wire_units();
    let expected: Vec<u16> = "abc".encode_utf16().collect();
    assert_eq!(units, expected);
}

#[test]
fn to_wire_units_non_bmp() {
    let field = CbPrefixedStringNoNull::new("\u{1F600}".to_owned());
    let units = field.to_wire_units();
    assert_eq!(units.as_ref(), &[0xD83Du16, 0xDE00u16]);
}

// ── size() matches encode() output length ────────────────────────────────

// Property: size() == encoded byte length for all five type variants, any string.
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]
    #[test]
    fn size_matches_encoded_length_prop(s in "\\PC{0,20}") {
        use ironrdp_core::Encode as _;
        let f1 = CchPrefixedString::new(s.clone());
        proptest::prop_assert_eq!(f1.size(), encode_vec(&f1).unwrap().len());

        let f2 = Cch32PrefixedString::new(s.clone());
        proptest::prop_assert_eq!(f2.size(), encode_vec(&f2).unwrap().len());

        let f3 = CbPrefixedStringNullExcluded::new(s.clone());
        proptest::prop_assert_eq!(f3.size(), encode_vec(&f3).unwrap().len());

        let f4 = CbPrefixedStringNullIncluded::new(s.clone());
        proptest::prop_assert_eq!(f4.size(), encode_vec(&f4).unwrap().len());

        let f5 = CbPrefixedStringNoNull::new(s.clone());
        proptest::prop_assert_eq!(f5.size(), encode_vec(&f5).unwrap().len());
    }
}
