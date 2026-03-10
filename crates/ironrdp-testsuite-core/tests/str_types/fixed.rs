use expect_test::expect;
use ironrdp_core::{DecodeOwned as _, ReadCursor, encode_vec};
use ironrdp_str::fixed::{FixedString, FixedStringBytesError};

// Property: encode + decode is a lossless round-trip for any string fitting in the field.
// "\\PC{0,7}" = printable non-control Unicode char; 7 chars worst-case (all non-BMP) = 14 code units ≤ WCHAR_COUNT-1=15.
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]
    #[test]
    fn round_trip_prop(s in "\\PC{0,7}") {
        let field = FixedString::<16>::new(s.clone()).unwrap();
        let encoded = encode_vec(&field).unwrap();
        proptest::prop_assert_eq!(encoded.len(), FixedString::<16>::WIRE_SIZE);
        let decoded = FixedString::<16>::decode_owned(&mut ReadCursor::new(&encoded)).unwrap();
        let native = decoded.to_native().unwrap();
        proptest::prop_assert_eq!(native.as_ref(), s.as_str());
    }
}

#[test]
fn round_trip_non_bmp() {
    // U+1F600 GRINNING FACE encodes as surrogate pair D83D DE00 = 2 code units.
    let original = "\u{1F600}";
    let s = FixedString::<4>::new(original).unwrap();
    let encoded = encode_vec(&s).unwrap();
    assert_eq!(encoded.len(), 8); // 4 * 2
    let decoded = FixedString::<4>::decode_owned(&mut ReadCursor::new(&encoded)).unwrap();
    assert_eq!(decoded.to_native().unwrap().as_ref(), original);
}

#[test]
fn wire_size_is_constant() {
    let empty = FixedString::<16>::new("").unwrap();
    let full = FixedString::<16>::new("a".repeat(15)).unwrap();
    use ironrdp_core::Encode as _;
    assert_eq!(empty.size(), 32);
    assert_eq!(full.size(), 32);
}

#[test]
fn rejects_overlong_string() {
    // WCHAR_COUNT=4 allows max 3 code units (slot 4 is for null).
    let err = FixedString::<4>::new("abcd").unwrap_err();
    expect![["
        StringTooLong {
            max_code_units: 3,
            actual_code_units: 4,
        }
    "]]
    .assert_debug_eq(&err);
}

#[test]
fn decode_strips_padding() {
    // Wire: [0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    // = 'A' (U+0041) followed by three null code units.
    let wire: &[u8] = &[0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let s = FixedString::<4>::decode_owned(&mut ReadCursor::new(wire)).unwrap();
    assert_eq!(s.to_native().unwrap().as_ref(), "A");
}

#[test]
fn decode_accepts_lone_surrogate_to_str_fails() {
    // Wire: lone high surrogate D800 followed by padding.
    // Decode succeeds (no eager validation); to_native() reports the error.
    let wire: &[u8] = &[0x00, 0xD8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    let s = FixedString::<4>::decode_owned(&mut ReadCursor::new(wire)).unwrap();
    let err = s.to_native().unwrap_err();
    expect![["
        InvalidUtf16
    "]]
    .assert_debug_eq(&err);
    // to_native_lossy() succeeds and replaces lone surrogates with U+FFFD.
    assert!(s.to_native_lossy().contains('\u{FFFD}'));
}

#[test]
fn non_bmp_code_units_counted_correctly() {
    // U+1F600 is 2 code units. In a WCHAR_COUNT=3 field, max content = 2 code units.
    assert!(FixedString::<3>::new("\u{1F600}").is_ok());
    // Two emoji = 4 code units, exceeds max of 2 for WCHAR_COUNT=3.
    let err = FixedString::<3>::new("\u{1F600}\u{1F600}").unwrap_err();
    expect![["
        StringTooLong {
            max_code_units: 2,
            actual_code_units: 4,
        }
    "]]
    .assert_debug_eq(&err);
}

// ── from_utf16le_bytes ────────────────────────────────────────────────────────

#[test]
fn from_utf16le_bytes_too_long_returns_err() {
    // 4 code units for WCHAR_COUNT=4 means 4 content units, but max is 3.
    let bytes: Vec<u8> = "abcd".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let err = FixedString::<4>::from_utf16le_bytes(&bytes).unwrap_err();
    expect![["
        StringTooLong(
            StringTooLong {
                max_code_units: 3,
                actual_code_units: 4,
            },
        )
    "]]
    .assert_debug_eq(&err);
    assert!(matches!(err, FixedStringBytesError::StringTooLong(_)));
}

#[test]
fn from_utf16le_bytes_odd_length_returns_err() {
    let err = FixedString::<4>::from_utf16le_bytes(&[0x41u8]).unwrap_err();
    expect![["
        OddByteCount
    "]]
    .assert_debug_eq(&err);
    assert_eq!(err, FixedStringBytesError::OddByteCount);
}

#[test]
fn from_utf16le_bytes_content_shorter_than_wire_size() {
    // Accepts content bytes shorter than WIRE_SIZE (32 bytes for WCHAR_COUNT=16).
    let bytes: Vec<u8> = "hi".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let s = FixedString::<16>::from_utf16le_bytes(&bytes).unwrap();
    assert_eq!(s.to_native().unwrap().as_ref(), "hi");
}

// ── from_wire_units / to_wire_units / into_wire_units ────────────────────────

#[test]
fn from_wire_units_round_trip() {
    let units: Vec<u16> = "hello".encode_utf16().collect();
    let s = FixedString::<16>::from_wire_units(units.clone()).unwrap();
    assert_eq!(s.to_native().unwrap().as_ref(), "hello");
    assert_eq!(s.to_wire_units().as_ref(), units.as_slice());
}

#[test]
fn from_wire_units_strips_trailing_nulls() {
    let mut units: Vec<u16> = "hi".encode_utf16().collect();
    units.push(0); // trailing null
    let s = FixedString::<8>::from_wire_units(units).unwrap();
    assert_eq!(s.to_native().unwrap().as_ref(), "hi");
}

#[test]
fn into_wire_units_from_decode() {
    // Decoded from wire bytes via utf16le_bytes_to_units — into_wire_units is zero-cost.
    let wire: &[u8] = &[0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]; // 'A' + padding
    let s = FixedString::<4>::decode_owned(&mut ReadCursor::new(wire)).unwrap();
    let units = s.into_wire_units();
    assert_eq!(units, &[0x0041u16]);
}

#[test]
fn to_wire_units_from_native() {
    let s = FixedString::<8>::new("abc").unwrap();
    let units = s.to_wire_units();
    let expected: Vec<u16> = "abc".encode_utf16().collect();
    assert_eq!(units.as_ref(), expected.as_slice());
}
