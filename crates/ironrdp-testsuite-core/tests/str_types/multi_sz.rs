use expect_test::expect;
use ironrdp_core::{DecodeOwned as _, ReadCursor, encode_vec};
use ironrdp_str::multi_sz::{MultiSzFlatError, MultiSzSegmentError, MultiSzString};

#[test]
fn empty_multi_sz() {
    // An empty MULTI_SZ: cch=1, one final null.
    let m = MultiSzString::new(core::iter::empty::<String>()).unwrap();
    let encoded = encode_vec(&m).unwrap();
    // 4 bytes (u32 cch=1) + 2 bytes (final null) = 6 bytes
    assert_eq!(encoded.len(), 6);
    assert_eq!(u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]), 1);
}

// ── new rejects embedded nulls ────────────────────────────────────────────────

#[test]
fn new_rejects_embedded_null() {
    assert!(MultiSzString::new(["ab\0c"]).is_err());
    assert!(MultiSzString::new(["ok", "bad\0"]).is_err());
}

// Property: new() → encode → decode gives back the original string list.
// Strings with embedded nulls are excluded: U+0000 is a segment delimiter in MULTI_SZ.
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]
    #[test]
    fn round_trip_prop(strings in proptest::collection::vec("[^\x00]{0,20}", 0..3usize)) {
        let m = MultiSzString::new(strings.clone()).unwrap();
        let encoded = encode_vec(&m).unwrap();
        let decoded = MultiSzString::decode_owned(&mut ReadCursor::new(&encoded)).unwrap();
        let result: Vec<String> = decoded.iter_native().map(|s| s.unwrap().into_owned()).collect();
        proptest::prop_assert_eq!(result, strings);
    }
}

#[test]
fn total_cch_counts_all_nulls() {
    // ["ab", "c"] -> total_cch = (2+1) + (1+1) + 1 = 6
    let m = MultiSzString::new(["ab", "c"]).unwrap();
    assert_eq!(m.total_cch(), 6);
}

// Property: size() == encoded byte length for any list of strings.
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]
    #[test]
    fn size_matches_encoded_length_prop(strings in proptest::collection::vec("[^\x00]{0,20}", 0..3usize)) {
        use ironrdp_core::Encode as _;
        let m = MultiSzString::new(strings).unwrap();
        proptest::prop_assert_eq!(m.size(), encode_vec(&m).unwrap().len());
    }
}

#[test]
fn rejects_missing_segment_null_terminator() {
    // Wire: cch=3, content = [o][f][sentinel] — the segment "of" has no per-string null before
    // the sentinel. After stripping the sentinel the stored units are [o, f], which ends with a
    // non-null unit. Without the new validation, iter_native / into_native would silently drop "of".
    let wire: &[u8] = &[
        0x03, 0x00, 0x00, 0x00, // u32 cch = 3
        0x6F, 0x00, // U+006F 'o'
        0x66, 0x00, // U+0066 'f'  (no per-string null terminator before the sentinel)
        0x00, 0x00, // final sentinel
    ];
    let err = MultiSzString::decode_owned(&mut ReadCursor::new(wire)).unwrap_err();
    expect![[r#"
        Error {
            context: "<ironrdp_str::multi_sz::MultiSzString as ironrdp_core::decode::DecodeOwned>::decode_owned",
            kind: InvalidField {
                field: "content",
                reason: "MULTI_SZ last segment is missing its null terminator",
            },
            source: None,
        }
    "#]]
    .assert_debug_eq(&err);
}

#[test]
fn rejects_zero_cch() {
    let wire: &[u8] = &[0x00, 0x00, 0x00, 0x00]; // cch=0
    let err = MultiSzString::decode_owned(&mut ReadCursor::new(wire)).unwrap_err();
    expect![[r#"
        Error {
            context: "<ironrdp_str::multi_sz::MultiSzString as ironrdp_core::decode::DecodeOwned>::decode_owned",
            kind: InvalidField {
                field: "cch",
                reason: "zero cch for MULTI_SZ is invalid",
            },
            source: None,
        }
    "#]]
    .assert_debug_eq(&err);
}

// ── from_utf16le_byte_strings ─────────────────────────────────────────────────

#[test]
fn from_utf16le_byte_strings_round_trip() {
    let byte_strings: Vec<Vec<u8>> = ["foo", "bar"]
        .iter()
        .map(|s| s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect())
        .collect();
    let m = MultiSzString::from_utf16le_byte_strings(byte_strings.iter().map(|v| v.as_slice())).unwrap();
    let strings: Vec<String> = m.iter_native().map(|s| s.unwrap().into_owned()).collect();
    assert_eq!(strings, ["foo", "bar"]);
}

#[test]
fn from_utf16le_byte_strings_odd_length_returns_err() {
    let err = MultiSzString::from_utf16le_byte_strings([&[0x41u8][..]]).unwrap_err();
    expect![["
        OddByteCount
    "]]
    .assert_debug_eq(&err);
    assert_eq!(err, MultiSzSegmentError::OddByteCount);
}

#[test]
fn from_utf16le_byte_strings_rejects_embedded_null() {
    // "a\0b" encoded as UTF-16LE: [0x61, 0x00, 0x00, 0x00, 0x62, 0x00]
    let segment: Vec<u8> = [0x61u16, 0x0000, 0x62].iter().flat_map(|u| u.to_le_bytes()).collect();
    let err = MultiSzString::from_utf16le_byte_strings([segment.as_slice()]).unwrap_err();
    expect![["
        EmbeddedNul
    "]]
    .assert_debug_eq(&err);
    assert_eq!(err, MultiSzSegmentError::EmbeddedNul);
}

// ── from_utf16le_flat ─────────────────────────────────────────────────────────

#[test]
fn from_utf16le_flat_round_trip() {
    // Flat content for ["foo", "bar"]: "foo\0bar\0\0" in UTF-16LE.
    let flat: Vec<u8> = "foo"
        .encode_utf16()
        .chain([0u16]) // per-string null
        .chain("bar".encode_utf16())
        .chain([0u16]) // per-string null
        .chain([0u16]) // sentinel
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let m = MultiSzString::from_utf16le_flat(&flat).unwrap();
    let strings: Vec<String> = m.iter_native().map(|s| s.unwrap().into_owned()).collect();
    assert_eq!(strings, ["foo", "bar"]);
}

#[test]
fn from_utf16le_flat_empty_list() {
    // Minimal flat content: just the sentinel null.
    let flat: &[u8] = &[0x00, 0x00];
    let m = MultiSzString::from_utf16le_flat(flat).unwrap();
    assert_eq!(m.iter_native().count(), 0);
}

#[test]
fn from_utf16le_flat_odd_length_returns_err() {
    let err = MultiSzString::from_utf16le_flat(&[0x00]).unwrap_err();
    expect![["
        OddByteCount
    "]]
    .assert_debug_eq(&err);
    assert_eq!(err, MultiSzFlatError::OddByteCount);
}

#[test]
fn from_utf16le_flat_missing_sentinel_returns_err() {
    // 'A' in UTF-16LE with no trailing null — the buffer does not end with 0x0000.
    let err = MultiSzString::from_utf16le_flat(&[0x41, 0x00]).unwrap_err();
    expect![["
        MissingSentinel
    "]]
    .assert_debug_eq(&err);
    assert_eq!(err, MultiSzFlatError::MissingSentinel);
}

#[test]
fn from_utf16le_flat_unterminated_last_segment_returns_err() {
    // [f, o, o, 0x0000]: the 0x0000 is treated as the sentinel; after stripping it,
    // the remaining ['f','o','o'] ends with 'o', not a per-string null terminator.
    let unterminated: Vec<u8> = "foo"
        .encode_utf16()
        .chain([0u16]) // sentinel (no per-string null precedes it)
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let err = MultiSzString::from_utf16le_flat(&unterminated).unwrap_err();
    expect![["
        UnterminatedLastSegment
    "]]
    .assert_debug_eq(&err);
    assert_eq!(err, MultiSzFlatError::UnterminatedLastSegment);
}

// ── from_wire_units_flat ──────────────────────────────────────────────────────

#[test]
fn from_wire_units_flat_round_trip() {
    let flat: Vec<u16> = "foo"
        .encode_utf16()
        .chain([0u16])
        .chain("bar".encode_utf16())
        .chain([0u16])
        .chain([0u16]) // sentinel
        .collect();
    let m = MultiSzString::from_wire_units_flat(flat).unwrap();
    let strings: Vec<String> = m.iter_native().map(|s| s.unwrap().into_owned()).collect();
    assert_eq!(strings, ["foo", "bar"]);
}

#[test]
fn from_wire_units_flat_empty_list() {
    let m = MultiSzString::from_wire_units_flat(vec![0u16]).unwrap();
    assert_eq!(m.iter_native().count(), 0);
}

#[test]
fn from_wire_units_flat_missing_sentinel_returns_err() {
    // Just 'A' with no trailing null — the buffer does not end with 0x0000.
    let err = MultiSzString::from_wire_units_flat(vec![0x0041u16]).unwrap_err();
    expect![["
        MissingSentinel
    "]]
    .assert_debug_eq(&err);
    assert_eq!(err, MultiSzFlatError::MissingSentinel);
}

#[test]
fn from_wire_units_flat_unterminated_last_segment_returns_err() {
    // [f, o, o, 0x0000]: the 0x0000 is treated as the sentinel; after stripping it,
    // the remaining ['f','o','o'] ends with 'o', not a per-string null terminator.
    let unterminated: Vec<u16> = "foo".encode_utf16().chain([0u16]).collect();
    let err = MultiSzString::from_wire_units_flat(unterminated).unwrap_err();
    expect![["
        UnterminatedLastSegment
    "]]
    .assert_debug_eq(&err);
    assert_eq!(err, MultiSzFlatError::UnterminatedLastSegment);
}

// ── from_unit_strings ─────────────────────────────────────────────────────────

#[test]
fn from_unit_strings_rejects_embedded_null() {
    // Interior null (0x0066 'f', 0x0000, 0x006F 'o') — rejected.
    let bad: Vec<Vec<u16>> = vec![vec![0x0066, 0x0000, 0x006F]]; // "f\0o"
    assert!(MultiSzString::from_unit_strings(bad).is_err());
}

#[test]
fn from_unit_strings_strips_trailing_null() {
    // A trailing null terminator is stripped; the string content is preserved.
    let with_null: Vec<u16> = "hi".encode_utf16().chain([0u16]).collect();
    let m = MultiSzString::from_unit_strings([with_null]).unwrap();
    assert_eq!(
        m.iter_native().map(|s| s.unwrap().into_owned()).collect::<Vec<_>>(),
        ["hi"]
    );
}

#[test]
fn from_unit_strings_trailing_null_only_not_treated_as_interior() {
    // A segment that is solely a null unit — stripped to empty string, not rejected.
    let only_null: Vec<u16> = vec![0u16];
    let m = MultiSzString::from_unit_strings([only_null]).unwrap();
    assert_eq!(
        m.iter_native().map(|s| s.unwrap().into_owned()).collect::<Vec<_>>(),
        [""]
    );
}

#[test]
fn from_unit_strings_round_trip() {
    let unit_strings: Vec<Vec<u16>> = ["foo", "bar"].iter().map(|s| s.encode_utf16().collect()).collect();
    let m = MultiSzString::from_unit_strings(unit_strings).unwrap();
    let strings: Vec<String> = m.iter_native().map(|s| s.unwrap().into_owned()).collect();
    assert_eq!(strings, ["foo", "bar"]);
}

#[test]
fn from_unit_strings_non_bmp() {
    let units: Vec<u16> = "\u{1F600}".encode_utf16().collect();
    let m = MultiSzString::from_unit_strings([units]).unwrap();
    let strings: Vec<String> = m.iter_native().map(|s| s.unwrap().into_owned()).collect();
    assert_eq!(strings, ["\u{1F600}"]);
}

#[test]
fn strings_lossy_replaces_lone_surrogates() {
    // Manually construct a MULTI_SZ with a lone high surrogate in one segment.
    // cch=3: [D800 LE][0000][0000] = lone surrogate + null + sentinel
    let wire: &[u8] = &[
        0x03, 0x00, 0x00, 0x00, // u32 cch = 3
        0x00, 0xD8, // lone high surrogate D800 (LE)
        0x00, 0x00, // null terminator
        0x00, 0x00, // final sentinel
    ];
    let decoded = MultiSzString::decode_owned(&mut ReadCursor::new(wire)).unwrap();
    // iter_native() returns Err for the segment with lone surrogate
    let err = decoded.iter_native().find_map(|r| r.err()).unwrap();
    expect![["
        InvalidUtf16
    "]]
    .assert_debug_eq(&err);
    // strings_lossy() replaces lone surrogate with U+FFFD
    let lossy: Vec<_> = decoded.iter_native_lossy().collect();
    assert_eq!(lossy.len(), 1);
    assert!(lossy[0].contains('\u{FFFD}'));
}
