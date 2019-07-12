use super::*;

#[test]
fn fast_path_header_with_long_len_is_parsed_correctly() {
    let buf = vec![0x9C, 0x81, 0xE7];

    let (fast_path, length) = parse_fast_path_header(&mut buf.as_slice()).unwrap();

    assert_eq!(fast_path.encryption_flags, 0x02);
    assert_eq!(fast_path.number_events, 7);
    assert_eq!(fast_path.length, 484);
    assert_eq!(length, 487);
}

#[test]
fn fast_path_header_with_short_len_is_parsed_correctly() {
    let buf = vec![0x8B, 0x08];

    let (fast_path, length) = parse_fast_path_header(&mut buf.as_slice()).unwrap();

    assert_eq!(fast_path.encryption_flags, 0x02);
    assert_eq!(fast_path.number_events, 2);
    assert_eq!(fast_path.length, 6);
    assert_eq!(length, 8);
}
