use super::*;

#[test]
fn write_sequence_tag_is_correct() {
    let mut buf = Vec::new();
    assert_eq!(write_sequence_tag(&mut buf, 0x100).unwrap(), 4);
    assert_eq!(buf, vec![0x30, 0x82, 0x01, 0x00]);
}

#[test]
fn read_sequence_tag_returns_correct_length() {
    let buf = vec![0x30, 0x82, 0x01, 0x00];
    assert_eq!(read_sequence_tag(&mut buf.as_slice()).unwrap(), 0x100);
}

#[test]
fn read_sequence_tag_returns_error_on_invalid_tag() {
    let buf = vec![0x3a, 0x82, 0x01, 0x00];
    assert_eq!(
        read_sequence_tag(&mut buf.as_slice()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn write_application_tag_is_correct_with_long_tag() {
    let mut buf = Vec::new();
    assert_eq!(write_application_tag(&mut buf, 0x1F, 0x0F).unwrap(), 3);
    assert_eq!(buf, vec![0x7F, 0x1F, 0x0F]);
}

#[test]
fn write_application_tag_is_correct_with_short_tag() {
    let mut buf = Vec::new();
    assert_eq!(write_application_tag(&mut buf, 0x08, 0x100).unwrap(), 4);
    assert_eq!(buf, vec![0x68, 0x82, 0x01, 0x00]);
}

#[test]
fn read_application_tag_is_correct_with_long_tag() {
    let buf = vec![0x7F, 0x1F, 0x0F];
    assert_eq!(read_application_tag(&mut buf.as_slice(), 0x1F).unwrap(), 0x0F);
}

#[test]
fn read_application_tag_is_correct_with_short_tag() {
    let buf = vec![0x68, 0x82, 0x01, 0x00];
    assert_eq!(read_application_tag(&mut buf.as_slice(), 0x08).unwrap(), 0x100);
}

#[test]
fn read_application_tag_returns_error_on_invalid_long_tag() {
    let buf = vec![0x68, 0x1B, 0x0F];
    assert_eq!(
        read_application_tag(&mut buf.as_slice(), 0x1F).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn read_application_tag_returns_error_on_invalid_long_tag_value() {
    let buf = vec![0x7F, 0x1B, 0x0F];
    assert_eq!(
        read_application_tag(&mut buf.as_slice(), 0x1F).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn read_application_tag_returns_error_on_invalid_short_tag() {
    let buf = vec![0x67, 0x0F];
    assert_eq!(
        read_application_tag(&mut buf.as_slice(), 0x08).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn write_enumerated_is_correct() {
    let mut buf = Vec::new();
    assert_eq!(write_enumerated(&mut buf, 0x0F).unwrap(), 3);
    assert_eq!(buf, vec![0x0A, 0x01, 0x0F]);
}

#[test]
fn read_enumerated_is_correct() {
    let buf = vec![0x0A, 0x01, 0x0F];
    assert_eq!(read_enumerated(&mut buf.as_slice(), 0x10).unwrap(), 0x0F);
}

#[test]
fn read_enumerated_returns_error_on_wrong_tag() {
    let buf = vec![0x0B, 0x01, 0x0F];
    assert_eq!(
        read_enumerated(&mut buf.as_slice(), 0x10).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn read_enumerated_returns_error_on_invalid_len() {
    let buf = vec![0x0A, 0x02, 0x0F];
    assert_eq!(
        read_enumerated(&mut buf.as_slice(), 0x10).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn read_enumerated_returns_error_on_invalid_variant() {
    let buf = vec![0x0A, 0x01, 0x0F];
    assert_eq!(
        read_enumerated(&mut buf.as_slice(), 0x05).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn write_bool_true_is_correct() {
    let mut buf = Vec::new();
    assert_eq!(write_bool(&mut buf, true).unwrap(), 3);
    assert_eq!(buf, vec![0x01, 0x01, 0xFF]);
}

#[test]
fn write_bool_false_is_correct() {
    let mut buf = Vec::new();
    assert_eq!(write_bool(&mut buf, false).unwrap(), 3);
    assert_eq!(buf, vec![0x01, 0x01, 0x00]);
}

#[test]
fn read_bool_true_is_correct() {
    let buf = vec![0x01, 0x01, 0xFF];
    assert_eq!(read_bool(&mut buf.as_slice()).unwrap(), true);
}

#[test]
fn read_bool_false_is_correct() {
    let buf = vec![0x01, 0x01, 0x00];
    assert_eq!(read_bool(&mut buf.as_slice()).unwrap(), false);
}

#[test]
fn read_bool_returns_error_on_wrong_tag() {
    let buf = vec![0x0A, 0x01, 0xFF];
    assert_eq!(
        read_bool(&mut buf.as_slice()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn read_bool_returns_error_on_invalid_len() {
    let buf = vec![0x01, 0x02, 0x0F];
    assert_eq!(
        read_bool(&mut buf.as_slice()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn write_octet_string_tag_is_correct() {
    let mut buf = Vec::new();
    assert_eq!(write_octet_string_tag(&mut buf, 0x0F).unwrap(), 2);
    assert_eq!(buf, vec![0x04, 0x0F]);
}

#[test]
fn read_octet_string_tag_is_correct() {
    let buf = vec![0x04, 0x0F];
    assert_eq!(read_octet_string_tag(&mut buf.as_slice()).unwrap(), 0x0F);
}

#[test]
fn read_octet_string_tag_returns_error_on_wrong_tag() {
    let buf = vec![0x05, 0x0F];
    assert_eq!(
        read_octet_string_tag(&mut buf.as_slice()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn write_octet_string_is_correct() {
    let mut buf = Vec::new();
    let string = [0x68, 0x65, 0x6c, 0x6c, 0x6f];
    let expected: Vec<_> = [0x04, 0x05].iter().chain(string.iter()).cloned().collect();
    assert_eq!(write_octet_string(&mut buf, &string).unwrap(), 7);
    assert_eq!(buf, expected);
}

#[test]
fn read_octet_string_is_correct() {
    let buf = vec![0x04, 0x03, 0x00, 0x01, 0x02];
    assert_eq!(read_octet_string(&mut buf.as_slice()).unwrap(), vec![0x00, 0x01, 0x02]);
}

#[test]
fn write_length_is_correct_with_3_byte_length() {
    let mut buf = Vec::new();
    assert_eq!(write_length(&mut buf, 0x100).unwrap(), 3);
    assert_eq!(buf, vec![0x82, 0x01, 0x00]);
}

#[test]
fn write_length_is_correct_with_2_byte_length() {
    let mut buf = Vec::new();
    assert_eq!(write_length(&mut buf, 0xFA).unwrap(), 2);
    assert_eq!(buf, vec![0x81, 0xFA]);
}

#[test]
fn write_length_is_correct_with_1_byte_length() {
    let mut buf = Vec::new();
    assert_eq!(write_length(&mut buf, 0x70).unwrap(), 1);
    assert_eq!(buf, vec![0x70]);
}

#[test]
fn read_length_is_correct_with_3_byte_length() {
    let buf = vec![0x82, 0x01, 0x00];
    assert_eq!(read_length(&mut buf.as_slice()).unwrap(), 0x100);
}

#[test]
fn read_length_is_correct_with_2_byte_length() {
    let buf = vec![0x81, 0xFA];
    assert_eq!(read_length(&mut buf.as_slice()).unwrap(), 0xFA);
}

#[test]
fn read_length_is_correct_with_1_byte_length() {
    let buf = vec![0x70];
    assert_eq!(read_length(&mut buf.as_slice()).unwrap(), 0x70);
}

#[test]
fn read_length_returns_error_on_invalid_length() {
    let buf = vec![0x8a, 0x1];
    assert_eq!(
        read_length(&mut buf.as_slice()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn write_integer_is_correct_with_4_byte_integer() {
    let mut buf = Vec::new();
    assert_eq!(write_integer(&mut buf, 0x0080_0000).unwrap(), 6);
    assert_eq!(buf, vec![0x02, 0x04, 0x00, 0x80, 0x00, 0x00]);
}

#[test]
fn write_integer_is_correct_with_3_byte_integer() {
    let mut buf = Vec::new();
    assert_eq!(write_integer(&mut buf, 0x80000).unwrap(), 5);
    assert_eq!(buf, vec![0x02, 0x03, 0x08, 0x00, 0x00]);
}

#[test]
fn write_integer_is_correct_with_2_byte_integer() {
    let mut buf = Vec::new();
    assert_eq!(write_integer(&mut buf, 0x800).unwrap(), 4);
    assert_eq!(buf, vec![0x02, 0x02, 0x08, 0x00]);
}

#[test]
fn write_integer_is_correct_with_1_byte_integer() {
    let mut buf = Vec::new();
    assert_eq!(write_integer(&mut buf, 0x79).unwrap(), 3);
    assert_eq!(buf, vec![0x02, 0x01, 0x79]);
}

#[test]
fn read_integer_is_correct_with_8_byte_integer() {
    let buf = vec![0x02, 0x08, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(read_integer(&mut buf.as_slice()).unwrap(), 0x0080_0000_0000_0000);
}

#[test]
fn read_integer_is_correct_with_4_byte_integer() {
    let buf = vec![0x02, 0x04, 0x00, 0x80, 0x00, 0x00];
    assert_eq!(read_integer(&mut buf.as_slice()).unwrap(), 0x0080_0000);
}

#[test]
fn read_integer_is_correct_with_3_byte_integer() {
    let buf = vec![0x02, 0x03, 0x08, 0x00, 0x00];
    assert_eq!(read_integer(&mut buf.as_slice()).unwrap(), 0x80000);
}

#[test]
fn read_integer_is_correct_with_2_byte_integer() {
    let buf = vec![0x02, 0x02, 0x08, 0x00];
    assert_eq!(read_integer(&mut buf.as_slice()).unwrap(), 0x800);
}

#[test]
fn read_integer_is_correct_with_1_byte_integer() {
    let buf = vec![0x02, 0x01, 0x79];
    assert_eq!(read_integer(&mut buf.as_slice()).unwrap(), 0x79);
}

#[test]
fn read_integer_returns_error_on_incorrect_len() {
    let buf = vec![0x02, 0x06, 0x79];
    assert_eq!(
        read_integer(&mut buf.as_slice()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn write_universal_tag_primitive_integer_is_correct() {
    let mut buf = Vec::new();
    assert_eq!(write_universal_tag(&mut buf, Tag::Integer, Pc::Primitive).unwrap(), 1);
    assert_eq!(buf, vec![0x02]);
}

#[test]
fn write_universal_tag_construct_enumerated_is_correct() {
    let mut buf = Vec::new();
    assert_eq!(
        write_universal_tag(&mut buf, Tag::Enumerated, Pc::Construct).unwrap(),
        1
    );
    assert_eq!(buf, vec![0x2A]);
}

#[test]
fn sizeof_length_with_long_len() {
    let len = 625;
    let expected = 3;
    assert_eq!(sizeof_length(len), expected);
}
