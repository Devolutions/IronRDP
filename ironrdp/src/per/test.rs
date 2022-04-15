use super::*;

#[test]
fn read_length_is_correct_length() {
    let buf = [0x05];

    let (length, sizeof_length) = read_length(buf.as_ref()).unwrap();

    assert_eq!(5, length);
    assert_eq!(buf.len(), sizeof_length);
}

#[test]
fn read_length_is_correct_long_length() {
    let buf = [0x80, 0x8d];

    let (length, sizeof_length) = read_length(buf.as_ref()).unwrap();

    assert_eq!(141, length);
    assert_eq!(buf.len(), sizeof_length);
}

#[test]
fn write_length_is_correct() {
    let expected_buf = vec![0x05];

    let mut buf = Vec::new();
    let size = write_length(&mut buf, 5).unwrap();

    assert_eq!(expected_buf, buf);
    assert_eq!(expected_buf.len(), size);
}

#[test]
fn write_length_is_correct_with_long_length() {
    let expected_buf = vec![0x80, 0x8d];

    let mut buf = Vec::new();
    let size = write_length(&mut buf, 141).unwrap();

    assert_eq!(expected_buf, buf);
    assert_eq!(expected_buf.len(), size);
}

#[test]
fn sizeof_length_is_correct_with_small_length() {
    assert_eq!(1, sizeof_length(10));
}

#[test]
fn sizeof_length_is_correct_with_long_length() {
    assert_eq!(2, sizeof_length(10_000));
}

#[test]
fn read_u32_returns_correct_with_null_number() {
    let buf = [0x00];
    assert_eq!(0, read_u32(buf.as_ref()).unwrap());
}

#[test]
fn read_u32_returns_correct_with_1_byte_number() {
    let buf = [0x01, 0x7f];
    assert_eq!(127, read_u32(buf.as_ref()).unwrap());
}

#[test]
fn read_u32_returns_correct_with_2_bytes_number() {
    let buf = [0x02, 0x7f, 0xff];
    assert_eq!(32767, read_u32(buf.as_ref()).unwrap());
}

#[test]
fn read_u32_returns_correct_with_4_bytes_number() {
    let buf = [0x04, 0x01, 0x12, 0xA8, 0x80];
    assert_eq!(18_000_000, read_u32(buf.as_ref()).unwrap());
}

#[test]
fn read_u32_fails_on_invalid_length() {
    let buf = [0x03, 0x01, 0x12, 0xA8, 0x80];
    assert!(read_u32(buf.as_ref()).is_err());
}

#[test]
fn write_u32_returns_correct_null_number() {
    let expected_buf = vec![0x01, 0x00];

    let mut buf = Vec::new();
    write_u32(&mut buf, 0).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn write_u32_returns_correct_1_byte_number() {
    let expected_buf = vec![0x01, 0x7f];

    let mut buf = Vec::new();
    write_u32(&mut buf, 127).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn write_u32_returns_correct_2_bytes_number() {
    let expected_buf = vec![0x02, 0x7f, 0xff];

    let mut buf = Vec::new();
    write_u32(&mut buf, 32767).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn write_u32_returns_correct_4_byte_number() {
    let expected_buf = vec![0x04, 0x01, 0x12, 0xA8, 0x80];

    let mut buf = Vec::new();
    write_u32(&mut buf, 18_000_000).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn read_u16_returns_correct_number() {
    let buf = [0x00, 0x07];
    assert_eq!(1008, read_u16(buf.as_ref(), 1001).unwrap());
}

#[test]
fn read_u16_fails_on_too_big_number_with_min_value() {
    let buf = [0xff, 0xff];

    match read_u16(buf.as_ref(), 1) {
        Err(ref e) if e.kind() == io::ErrorKind::InvalidData => (),
        _ => panic!("Invalid result for read_u16"),
    };
}

#[test]
fn write_u16_returns_correct_number() {
    let expected_buf = vec![0x00, 0x07];

    let mut buf = Vec::new();
    write_u16(&mut buf, 1008, 1001).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn write_u16_fails_if_min_is_greater_then_number() {
    let mut buf = Vec::new();
    match write_u16(&mut buf, 1000, 1001) {
        Err(ref e) if e.kind() == io::ErrorKind::InvalidInput => (),
        _ => panic!("Invalid result for write_u16"),
    };
}

#[test]
fn read_object_id_returns_ok() {
    let buf = [0x05, 0x00, 0x14, 0x7c, 0x00, 0x01];
    assert_eq!([0, 0, 20, 124, 0, 1], read_object_id(buf.as_ref()).unwrap());
}

#[test]
fn write_object_id_is_correct() {
    let expected_buf = vec![0x05, 0x00, 0x14, 0x7c, 0x00, 0x01];
    let mut buf = Vec::new();

    write_object_id(&mut buf, [0, 0, 20, 124, 0, 1]).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn read_enum_fails_on_invalid_enum_with_count() {
    let buf = [0x05];

    match read_enum(buf.as_ref(), 1) {
        Err(ref e) if e.kind() == io::ErrorKind::InvalidData => (),
        _ => panic!("Invalid result for read_enum"),
    };
}

#[test]
fn read_enum_returns_correct_enum() {
    let buf = [0x05];

    assert_eq!(5, read_enum(buf.as_ref(), 10).unwrap());
}

#[test]
fn read_enum_fails_on_max_number() {
    let buf = [0xff];

    match read_enum(buf.as_ref(), 0xff) {
        Err(ref e) if e.kind() == io::ErrorKind::InvalidData => (),
        _ => panic!("Invalid result for read_enum"),
    };
}

#[test]
fn read_numeric_string_returns_ok() {
    let buf = [0x00, 0x10];
    read_numeric_string(buf.as_ref(), 1).unwrap();
}

#[test]
fn write_numeric_string_is_correct() {
    let expected_buf = vec![0x00, 0x10];
    let octet_string = b"1";
    let mut buf = Vec::new();

    write_numeric_string(&mut buf, octet_string.as_ref(), 1).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn read_octet_string_returns_ok() {
    let buf = [0x00, 0x44, 0x75, 0x63, 0x61];
    assert_eq!(b"Duca".as_ref(), read_octet_string(buf.as_ref(), 4).unwrap().as_slice());
}

#[test]
fn write_octet_string_is_correct() {
    let expected_buf = vec![0x00, 0x44, 0x75, 0x63, 0x61];
    let octet_string = b"Duca";

    let mut buf = Vec::new();
    write_octet_string(&mut buf, octet_string.as_ref(), 4).unwrap();

    assert_eq!(expected_buf, buf);
}
