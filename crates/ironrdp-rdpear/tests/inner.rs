use ironrdp_core::{Decode as _, Encode as _, ReadCursor, WriteCursor, encode_vec};
use ironrdp_dvc as _;
use ironrdp_pdu as _;
use ironrdp_rdpear::RemoteGuardInnerPacket;
use ironrdp_svc as _;

const PACKET: &[u8] = &[
    0x30, 0x24, // SEQUENCE
    0xa1, 0x0a, 0x04, 0x08, // [1] OCTET STRING
    b'N', 0, b'T', 0, b'L', 0, b'M', 0, 0xa2, 0x16, 0x04, 0x14, // [2] OCTET STRING
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // package header
    1, 2, 3, 4, // opaque package call, not an NDR fixture
];

#[test]
fn canonical_packet_matches_independent_der_vector() {
    let mut cursor = ReadCursor::new(PACKET);
    let packet = RemoteGuardInnerPacket::decode(&mut cursor).expect("canonical packet");
    assert!(cursor.is_empty());
    assert_eq!(packet.package_name, [b'N', 0, b'T', 0, b'L', 0, b'M', 0]);
    assert_eq!(packet.package_call, [1, 2, 3, 4]);
    assert_eq!(packet.package_name.as_ptr(), PACKET[6..].as_ptr());
    assert_eq!(packet.package_call.as_ptr(), PACKET[34..].as_ptr());
    assert_eq!(packet.size(), PACKET.len());
    assert_eq!(encode_vec(&packet).expect("encode"), PACKET);
}

#[test]
fn rejects_every_truncation_and_trailing_data() {
    for end in 0..PACKET.len() {
        assert!(
            RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&PACKET[..end])).is_err(),
            "prefix {end}"
        );
    }
    let mut bytes = PACKET.to_vec();
    bytes.push(0);
    assert!(RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&bytes)).is_err());
}

#[test]
fn rejects_invalid_tags_and_explicit_field_lengths() {
    for (index, value) in [
        (0, 0x31),
        (2, 0xa2),
        (4, 0x24),
        (14, 0xa1),
        (16, 0x24),
        (3, 9),
        (3, 11),
        (5, 7),
        (5, 9),
        (15, 21),
        (15, 23),
        (17, 19),
        (17, 21),
    ] {
        let mut bytes = PACKET.to_vec();
        bytes[index] = value;
        assert!(
            RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&bytes)).is_err(),
            "field {index} value {value}"
        );
    }
}

#[test]
fn rejects_noncanonical_and_unsupported_versions() {
    for version in [0, 1, 0xff] {
        let mut bytes = PACKET[..2].to_vec();
        bytes[1] += 5;
        bytes.extend_from_slice(&[0xa0, 3, 0x0a, 1, version]);
        bytes.extend_from_slice(&PACKET[2..]);
        let error = RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&bytes)).expect_err("invalid version");
        if version == 0 {
            assert!(error.to_string().contains("default version must be omitted"));
        } else {
            assert!(error.to_string().contains("version"));
        }
    }
}

#[test]
fn rejects_indefinite_nonminimal_and_oversized_lengths() {
    for length_bytes in [
        &[0x80][..],
        &[0x81, 0x24],
        &[0x82, 0, 0x24],
        &[0x89, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        &[0x88, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        &[0xff],
    ] {
        let mut bytes = vec![0x30];
        bytes.extend_from_slice(length_bytes);
        bytes.extend_from_slice(&PACKET[2..]);
        assert!(
            RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&bytes)).is_err(),
            "length {length_bytes:?}"
        );
    }

    for index in [3, 5, 15, 17] {
        let mut bytes = PACKET.to_vec();
        bytes[1] += 1;
        bytes[index] = 0x81;
        bytes.insert(index + 1, PACKET[index]);
        assert!(RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&bytes)).is_err());
    }
}

#[test]
fn rejects_each_invalid_package_header_byte_and_short_header() {
    for index in 18..34 {
        let mut bytes = PACKET.to_vec();
        bytes[index] ^= 0xff;
        assert!(
            RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&bytes)).is_err(),
            "header byte {index}"
        );
    }
    let mut bytes = PACKET[..33].to_vec();
    bytes[1] = 31;
    bytes[15] = 17;
    bytes[17] = 15;
    assert!(RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&bytes)).is_err());
}

#[test]
fn rejects_extensions_and_duplicate_fields_explicitly() {
    for extension in [&[0xa3, 2, 5, 0][..], &PACKET[2..14]] {
        let mut bytes = PACKET.to_vec();
        bytes[1] += u8::try_from(extension.len()).expect("small fixture");
        bytes.extend_from_slice(extension);
        let error = RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&bytes)).expect_err("unsupported extension");
        assert!(error.to_string().contains("extension tag"));
    }
}

#[test]
fn plaintext_limit_is_checked_before_parsing() {
    for limit in [0, PACKET.len() - 1] {
        let mut cursor = ReadCursor::new(PACKET);
        let error = RemoteGuardInnerPacket::decode_with_limit(&mut cursor, limit).expect_err("exceeds limit");
        assert!(error.to_string().contains("exceeds configured limit"));
        assert_eq!(cursor.pos(), 0);
    }
    for limit in [PACKET.len(), PACKET.len() + 1] {
        let mut cursor = ReadCursor::new(PACKET);
        let packet = RemoteGuardInnerPacket::decode_with_limit(&mut cursor, limit).expect("within limit");
        assert_eq!(packet.package_call, [1, 2, 3, 4]);
        assert!(cursor.is_empty());
    }
}

#[test]
fn long_der_lengths_match_independent_boundary_headers() {
    for (call_length, sequence_header, buffer_header) in [
        (111, &[0x30, 0x81, 0x90][..], &[0xa2, 0x81, 0x81, 0x04, 0x7f][..]),
        (112, &[0x30, 0x81, 0x92], &[0xa2, 0x81, 0x83, 0x04, 0x81, 0x80]),
        (239, &[0x30, 0x82, 1, 0x12], &[0xa2, 0x82, 1, 2, 0x04, 0x81, 0xff]),
        (240, &[0x30, 0x82, 1, 0x14], &[0xa2, 0x82, 1, 4, 0x04, 0x82, 1, 0]),
    ] {
        let call = vec![0x5a; call_length];
        let packet = RemoteGuardInnerPacket {
            package_name: &PACKET[6..14],
            package_call: &call,
        };
        let encoded = encode_vec(&packet).expect("encode");
        assert_eq!(&encoded[..sequence_header.len()], sequence_header);
        assert_eq!(
            &encoded[sequence_header.len() + 12..sequence_header.len() + 12 + buffer_header.len()],
            buffer_header
        );
        assert_eq!(packet.size(), encoded.len());
        let decoded = RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&encoded)).expect("long packet");
        assert_eq!(decoded, packet);
        assert_eq!(decoded.package_call, call);
    }
}

#[test]
fn empty_octet_strings_remain_opaque_not_authorized() {
    let encoded = encode_vec(&RemoteGuardInnerPacket {
        package_name: &[],
        package_call: &[],
    })
    .expect("encode");
    assert_eq!(
        encoded,
        [
            0x30, 0x18, 0xa1, 2, 4, 0, 0xa2, 0x12, 4, 0x10, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    let decoded = RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&encoded)).expect("valid opaque fields");
    assert!(decoded.package_name.is_empty());
    assert!(decoded.package_call.is_empty());
}

#[test]
fn short_encode_buffer_is_not_modified() {
    let packet = RemoteGuardInnerPacket::decode(&mut ReadCursor::new(PACKET)).expect("fixture");
    let mut buffer = vec![0x55; packet.size() - 1];
    let mut cursor = WriteCursor::new(&mut buffer);
    assert!(packet.encode(&mut cursor).is_err());
    assert_eq!(cursor.pos(), 0);
    assert!(buffer.iter().all(|byte| *byte == 0x55));
}

#[test]
fn diagnostics_do_not_expose_inner_data() {
    let packet = RemoteGuardInnerPacket {
        package_name: b"sensitive-package-marker",
        package_call: b"secret-call-marker",
    };
    let debug = format!("{packet:?}");
    assert!(!debug.contains("sensitive-package-marker"));
    assert!(!debug.contains("secret-call-marker"));
    assert!(debug.contains("package_call_length"));

    let mut encoded = encode_vec(&packet).expect("encode");
    encoded.push(0);
    let error = RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&encoded)).expect_err("trailing byte");
    let message = format!("{error:?} {error}");
    assert!(!message.contains("sensitive-package-marker"));
    assert!(!message.contains("secret-call-marker"));
}
