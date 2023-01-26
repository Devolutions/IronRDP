use super::*;

#[test]
fn tpkt_header_is_written_correctly() {
    let expected = [
        0x3, // version
        0x0, // reserved
        0x5, 0x42, // lenght in BE
    ];
    let mut buff = Vec::new();

    let tpkt_header = TpktHeader::new(0x542);

    tpkt_header.to_buffer(&mut buff).unwrap();

    assert_eq!(buff, expected);
}

#[test]
fn tpkt_header_is_read_correctly() {
    let stream = [
        0x3, // version
        0x0, // reserved
        0x5, 0x42, // lenght in BE
    ];

    let correct_tpkt_header = TpktHeader::new(0x542);

    assert_eq!(correct_tpkt_header, TpktHeader::from_buffer(stream.as_ref()).unwrap());
}

#[test]
fn buffer_length_is_correct_for_tpkt_header() {
    let stream = [
        0x3, // version
        0x0, // reserved
        0x5, 0x42, // lenght in BE
    ];

    let tpkt_header = TpktHeader::new(0x542);

    assert_eq!(stream.len(), tpkt_header.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_data() {
    #[rustfmt::skip]
    let buffer = [
        0x03u8, 0x00, 0x00, 0x0c, // tpkt
        0x02, 0xf0, 0x80, // data tpdu
        0x04, 0x01, 0x00, 0x01, 0x00,
    ];

    let mut buffer_slice = &buffer[..];

    let data_header = DataHeader { data_length: 5 };

    assert_eq!(data_header, DataHeader::from_buffer(&mut buffer_slice).unwrap());
    assert_eq!(5, buffer_slice.len());
}

#[test]
fn to_buffer_correctly_serializes_data() {
    #[rustfmt::skip]
    let expected = [
        0x03u8, 0x00, 0x00, 0x0c, // tpkt
        0x02, 0xf0, 0x80, // data tpdu
    ];

    let data_header = DataHeader { data_length: 5 };

    let mut buffer = Vec::new();
    data_header.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, expected.as_ref());
}

#[test]
fn buffer_length_is_correct_for_data() {
    #[rustfmt::skip]
    let buffer = [
        0x03u8, 0x00, 0x00, 0x0c, // tpkt
        0x02, 0xf0, 0x80, // data tpdu
    ];

    let data_header = DataHeader { data_length: 5 };

    assert_eq!(buffer.len(), data_header.buffer_length());
}
