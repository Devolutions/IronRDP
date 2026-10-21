use expect_test::expect;
use ironrdp_core::{DecodeResult, ReadCursor, read_padding};

#[test]
fn read_padding_macro_returns_error_on_truncated_input() {
    fn decode_with_trailing_padding(src: &mut ReadCursor<'_>) -> DecodeResult<u8> {
        let value = src.read_u8();
        read_padding!(src, 4);
        Ok(value)
    }

    let mut src = ReadCursor::new(&[1, 0, 0, 0]);

    let e = decode_with_trailing_padding(&mut src).unwrap_err();

    assert_eq!(src.pos(), 1);
    assert_eq!(src.len(), 3);
    expect![[r#"
        Error {
            context: "integration_tests_core::padding::read_padding_macro_returns_error_on_truncated_input::decode_with_trailing_padding",
            kind: NotEnoughBytes {
                received: 3,
                expected: 4,
                offset: Some(
                    1,
                ),
            },
            source: None,
        }
    "#]]
    .assert_debug_eq(&e);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "advancing past the end of the buffer")]
fn read_cursor_advance_past_end_is_caught_in_debug_builds() {
    let mut src = ReadCursor::new(&[0, 0]);

    src.advance(3);
}

/// Release builds have no debug assertion: an overrun cursor reads as empty instead of overflowing.
#[test]
#[cfg(not(debug_assertions))]
fn read_cursor_len_saturates_when_advanced_past_end() {
    let mut src = ReadCursor::new(&[0, 0]);

    src.advance(3);

    assert_eq!(src.len(), 0);
    assert!(src.is_empty());
    assert!(src.remaining().is_empty());
    assert!(src.try_read_u8().is_err());
}
