use ironrdp_rdpeudp::pdu::{PacketPrefixByte, PrefixError, decode_with_prefix, encode_with_prefix};

#[test]
fn prefix_byte_normal() {
    let prefix = PacketPrefixByte::normal(10);
    assert_eq!(prefix.packet_type_index, 0);
    assert_eq!(prefix.short_packet_length, 7);
    assert!(!prefix.is_dummy());
}

#[test]
fn prefix_byte_dummy() {
    let prefix = PacketPrefixByte::dummy(10);
    assert_eq!(prefix.packet_type_index, 8);
    assert_eq!(prefix.short_packet_length, 7);
    assert!(prefix.is_dummy());
}

#[test]
fn prefix_byte_short_packet() {
    let prefix = PacketPrefixByte::normal(3);
    assert_eq!(prefix.short_packet_length, 3);
}

#[test]
fn roundtrip_normal_packet() {
    let packet = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22, 0x33];
    let mut wire_buf = Vec::new();
    let wire_len = encode_with_prefix(&packet, false, &mut wire_buf).expect("encode");
    assert_eq!(wire_buf.len(), wire_len);

    let (prefix, decoded_packet) = decode_with_prefix(&mut wire_buf).expect("decode");
    assert!(!prefix.is_dummy());
    assert_eq!(decoded_packet, &*packet);
}

#[test]
fn roundtrip_dummy_packet() {
    let packet = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    let mut wire_buf = Vec::new();
    encode_with_prefix(&packet, true, &mut wire_buf).expect("encode");

    let (prefix, decoded_packet) = decode_with_prefix(&mut wire_buf).expect("decode");
    assert!(prefix.is_dummy());
    assert_eq!(decoded_packet, &*packet);
}

#[test]
fn roundtrip_short_packet() {
    // Packet shorter than 7 bytes gets padded, then unpadded on receive.
    let packet = vec![0xAA, 0xBB, 0xCC]; // 3 bytes
    let mut wire_buf = Vec::new();
    encode_with_prefix(&packet, false, &mut wire_buf).expect("encode");

    // Wire should be 8 bytes: 1 prefix + 7 (padded from 3).
    assert_eq!(wire_buf.len(), 8);

    let (prefix, decoded_packet) = decode_with_prefix(&mut wire_buf).expect("decode");
    assert_eq!(prefix.short_packet_length, 3);
    assert_eq!(decoded_packet, &*packet);
}

#[test]
fn roundtrip_minimum_packet() {
    // Minimum: 2 bytes (the v2 header alone).
    let packet = vec![0x05, 0x60];
    let mut wire_buf = Vec::new();
    encode_with_prefix(&packet, false, &mut wire_buf).expect("encode");

    let (prefix, decoded_packet) = decode_with_prefix(&mut wire_buf).expect("decode");
    assert_eq!(prefix.short_packet_length, 2);
    assert_eq!(decoded_packet, &*packet);
}

#[test]
fn roundtrip_large_packet() {
    // Near-MTU packet.
    let packet: Vec<u8> = (0..1200u32)
        .map(|i| u8::try_from(i % 256).expect("modulo 256 fits in u8"))
        .collect();
    let mut wire_buf = Vec::new();
    encode_with_prefix(&packet, false, &mut wire_buf).expect("encode");

    let (prefix, decoded_packet) = decode_with_prefix(&mut wire_buf).expect("decode");
    assert!(!prefix.is_dummy());
    assert_eq!(prefix.short_packet_length, 7);
    assert_eq!(decoded_packet, &*packet);
}

#[test]
fn decode_payload_too_short() {
    let mut wire = vec![0x00, 0x01, 0x02]; // only 3 bytes, need 8
    let result = decode_with_prefix(&mut wire);
    assert!(matches!(result, Err(PrefixError::PayloadTooShort { .. })));
}
