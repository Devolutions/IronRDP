use super::*;

#[test]
fn tpkt_header_is_written_correctly() {
    let expected = [
        0x3, // version
        0x0, // reserved
        0x5, 0x42, // lenght in BE
    ];
    let mut buff = Vec::new();

    write_tpkt_header(&mut buff, 1346).unwrap();

    assert_eq!(buff, expected);
}

#[test]
fn tpdu_header_non_data_is_written_correctly() {
    let length = 0x42;
    let code = X224TPDUType::ConnectionRequest;
    let expected = [
        length - 1,
        code.to_u8().unwrap(),
        0x0,
        0x0, // DST-REF
        0x0,
        0x0, // SRC-REF
        0x0, // Class 0
    ];
    let mut buff = Vec::new();

    write_tpdu_header(&mut buff, length, code, 0).unwrap();

    assert_eq!(buff, expected);
}

#[test]
fn tpdu_header_data_is_written_correctly() {
    let length = 0x42;
    let code = X224TPDUType::Data;
    let expected = [
        2,
        code.to_u8().unwrap(),
        0x80, // EOT
    ];
    let mut buff = Vec::new();

    write_tpdu_header(&mut buff, length, code, 0).unwrap();

    assert_eq!(buff, expected);
}

#[test]
fn tpdu_code_and_len_are_read_correctly() {
    let expected_length = 0x42;
    let expected_code = X224TPDUType::ConnectionRequest;
    let stream = [
        expected_length,
        expected_code.to_u8().unwrap(),
        0x0,
        0x0, // DST-REF
        0x0,
        0x0, // SRC-REF
        0x0, // Class 0
    ];

    let (length, code) = parse_tdpu_header(&mut stream.as_ref()).unwrap();

    assert_eq!(length, expected_length);
    assert_eq!(code, expected_code);
}

#[test]
fn parse_tdpu_non_data_header_advance_stream_position() {
    let expected_length = 0x42;
    let expected_code = X224TPDUType::ConnectionRequest;
    let stream = [
        expected_length,
        expected_code.to_u8().unwrap(),
        0x0,
        0x0, // DST-REF
        0x0,
        0x0, // SRC-REF
        0x0, // Class 0
        0xbf,
    ];
    let mut slice = stream.as_ref();

    parse_tdpu_header(&mut slice).unwrap();

    let next = slice.read_u8().unwrap();
    assert_eq!(next, 0xbf);
}

#[test]
fn parse_tdpu_data_header_advance_stream_position() {
    let expected_length = 0x42;
    let expected_code = X224TPDUType::Data;
    let stream = [
        expected_length,
        expected_code.to_u8().unwrap(),
        0x80, // EOT
        0xbf,
    ];
    let mut slice = stream.as_ref();

    parse_tdpu_header(&mut slice).unwrap();

    let next = slice.read_u8().unwrap();
    assert_eq!(next, 0xbf);
}

#[test]
fn decode_x224_correctly_decodes_connection_request() {
    let tpkt_tpdu_header = [
        0x03, 0x00, 0x00, 0x2c, 0x27, 0xe0, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let expected_tpdu = [
        0x43, 0x6f, 0x6f, 0x6b, 0x69, 0x65, 0x3a, 0x20, 0x6d, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
        0x68, 0x3d, 0x65, 0x6c, 0x74, 0x6f, 0x6e, 0x73, 0x0d, 0x0a, 0x01, 0x00, 0x08, 0x00, 0x00,
        0x00, 0x00, 0x00,
    ];
    let mut stream = BytesMut::with_capacity(tpkt_tpdu_header.len() + expected_tpdu.len());
    stream.extend_from_slice(&tpkt_tpdu_header);
    stream.extend_from_slice(&expected_tpdu);

    let (code, tpdu) = decode_x224(&mut stream).unwrap();

    assert_eq!(code, X224TPDUType::ConnectionRequest);
    assert_eq!(tpdu.as_ref(), expected_tpdu.as_ref());
}

#[test]
fn decode_x224_correctly_decodes_connection_confirm() {
    let tpkt_tpdu_header = [
        0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0, 0x00, 0x00, 0x12, 0x34, 0x00,
    ];
    let expected_tpdu = [0x02, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00];
    let mut stream = BytesMut::with_capacity(tpkt_tpdu_header.len() + expected_tpdu.len());
    stream.extend_from_slice(&tpkt_tpdu_header);
    stream.extend_from_slice(&expected_tpdu);

    let (code, tpdu) = decode_x224(&mut stream).unwrap();

    assert_eq!(code, X224TPDUType::ConnectionConfirm);
    assert_eq!(tpdu.as_ref(), expected_tpdu.as_ref());
}

#[test]
fn decode_x224_correctly_decodes_data() {
    let tpkt_tpdu_header = [0x03, 0x00, 0x00, 0x0c, 0x02, 0xf0, 0x80];
    let expected_tpdu = [0x04, 0x01, 0x00, 0x01, 0x00];
    let mut stream = BytesMut::with_capacity(tpkt_tpdu_header.len() + expected_tpdu.len());
    stream.extend_from_slice(&tpkt_tpdu_header);
    stream.extend_from_slice(&expected_tpdu);

    let (code, tpdu) = decode_x224(&mut stream).unwrap();

    assert_eq!(code, X224TPDUType::Data);
    assert_eq!(tpdu.as_ref(), expected_tpdu.as_ref());
}

#[test]
fn decode_x224_fails_on_incorrect_tpkt_len() {
    let mut stream = vec![
        0x03, 0x00, 0x00, 0x00, 0x02, 0xf0, 0x80, 0x04, 0x01, 0x00, 0x01, 0x00,
    ]
    .into();

    match decode_x224(&mut stream) {
        Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => (),
        Err(_e) => panic!("wrong error type"),
        _ => panic!("error expected"),
    };
}
