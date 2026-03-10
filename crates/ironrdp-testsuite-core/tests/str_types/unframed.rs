use expect_test::expect;
use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_str::unframed::UnframedString;

fn make_utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

#[test]
fn decode_by_wchar_count() {
    let wire = make_utf16le("hello");
    let s = UnframedString::decode(&mut ReadCursor::new(&wire), 5).unwrap();
    assert_eq!(s.to_native().unwrap().as_ref(), "hello");
}

#[test]
fn decode_from_byte_len() {
    let wire = make_utf16le("hi");
    let s = UnframedString::decode_from_byte_len(&mut ReadCursor::new(&wire), 4).unwrap();
    assert_eq!(s.to_native().unwrap().as_ref(), "hi");
}

#[test]
fn rejects_odd_byte_len() {
    let wire = make_utf16le("hi");
    let err = UnframedString::decode_from_byte_len(&mut ReadCursor::new(&wire), 3).unwrap_err();
    expect![[r#"
        Error {
            context: "ironrdp_str::unframed::UnframedString::decode_from_byte_len",
            kind: InvalidField {
                field: "byte_len",
                reason: "odd byte count for utf-16 string field",
            },
            source: None,
        }
    "#]]
    .assert_debug_eq(&err);
}

#[test]
fn strips_trailing_null() {
    // Wire with null terminator included in the count.
    let mut wire = make_utf16le("hello");
    wire.extend_from_slice(&[0x00, 0x00]); // null
    let s = UnframedString::decode(&mut ReadCursor::new(&wire), 6).unwrap();
    assert_eq!(s.to_native().unwrap().as_ref(), "hello");
}

#[test]
fn non_bmp_round_trip() {
    let original = "\u{1F600}";
    let s = UnframedString::new(original.to_owned());
    assert_eq!(s.utf16_len(), 2);
    assert_eq!(s.wire_size(), 4);

    let mut buf = vec![0u8; s.wire_size()];
    s.encode_into(&mut WriteCursor::new(&mut buf)).unwrap();
    let decoded = UnframedString::decode(&mut ReadCursor::new(&buf), 2).unwrap();
    assert_eq!(decoded.to_native().unwrap().as_ref(), original);
}

#[test]
fn lone_surrogate_decode_succeeds_to_native_fails() {
    // Lone high surrogate D800 LE. Decode no longer validates; to_native() reports error.
    let wire: &[u8] = &[0x00, 0xD8];
    let decoded = UnframedString::decode(&mut ReadCursor::new(wire), 1).unwrap();
    let err = decoded.to_native().unwrap_err();
    expect![["
        InvalidUtf16
    "]]
    .assert_debug_eq(&err);
    assert!(decoded.to_native_lossy().contains('\u{FFFD}'));
}

// Property: wire_size() is always the exact number of bytes encode_into() requires.
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]
    #[test]
    fn wire_size_prop(s in "\\PC{0,20}") {
        let f = UnframedString::new(s);
        let mut buf = vec![0u8; f.wire_size()];
        proptest::prop_assert!(f.encode_into(&mut WriteCursor::new(&mut buf)).is_ok());
    }
}

// ── from_utf16le_bytes ────────────────────────────────────────────────────────

#[test]
fn from_utf16le_bytes_strips_trailing_null() {
    let mut wire = make_utf16le("hi");
    wire.extend_from_slice(&[0x00, 0x00]);
    let s = UnframedString::from_utf16le_bytes(&wire).unwrap();
    assert_eq!(s.to_native().unwrap().as_ref(), "hi");
}

// ── from_wire_units / to_wire_units / into_wire_units ────────────────────────

#[test]
fn from_wire_units_round_trip() {
    let units: Vec<u16> = "hello".encode_utf16().collect();
    let s = UnframedString::from_wire_units(units.clone());
    assert_eq!(s.to_native().unwrap().as_ref(), "hello");
    assert_eq!(s.to_wire_units().as_ref(), units.as_slice());
}

#[test]
fn from_wire_units_strips_trailing_null() {
    let mut units: Vec<u16> = "hi".encode_utf16().collect();
    units.push(0);
    let s = UnframedString::from_wire_units(units);
    assert_eq!(s.to_native().unwrap().as_ref(), "hi");
}

#[test]
fn into_wire_units_from_decode() {
    let wire = make_utf16le("abc");
    let s = UnframedString::decode(&mut ReadCursor::new(&wire), 3).unwrap();
    let units = s.into_wire_units();
    let expected: Vec<u16> = "abc".encode_utf16().collect();
    assert_eq!(units, expected);
}

#[test]
fn to_wire_units_from_native_encodes_correctly() {
    let s = UnframedString::new("\u{1F600}");
    let units = s.to_wire_units();
    assert_eq!(units.as_ref(), &[0xD83Du16, 0xDE00u16]);
}
