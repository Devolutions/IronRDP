use ironrdp_core::ReadCursor;

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
