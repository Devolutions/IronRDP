use expect_test::expect;
use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::nego::{
    ConnectionConfirm, ConnectionRequest, ConnectionRequestWithOpaqueRoutingToken, Cookie, CorrelationInfo,
    FailureCode, NegoRequestData, OpaqueRoutingToken, RequestFlags, ResponseFlags, RoutingToken, SecurityProtocol,
};
use ironrdp_pdu::tpdu::{TpduCode, TpduHeader};
use ironrdp_pdu::tpkt::TpktHeader;
use ironrdp_pdu::x224::{X224, user_data_size};
use ironrdp_testsuite_core::encode_decode_test;

const SAMPLE_TPKT_HEADER_BINARY: [u8; 4] = [
    0x3, // version
    0x0, // reserved
    0x5, 0x42, // length in BE
];

const SAMPLE_TPKT_HEADER: TpktHeader = TpktHeader { packet_length: 0x542 };

#[test]
fn tpkt_header_write() {
    let mut buffer = [0; 4];
    let mut cursor = WriteCursor::new(&mut buffer);
    SAMPLE_TPKT_HEADER.write(&mut cursor).unwrap();
    assert_eq!(cursor.inner(), SAMPLE_TPKT_HEADER_BINARY);
}

#[test]
fn tpkt_header_read() {
    let mut cursor = ReadCursor::new(&SAMPLE_TPKT_HEADER_BINARY);
    let tpkt = TpktHeader::read(&mut cursor).unwrap();
    assert_eq!(tpkt, SAMPLE_TPKT_HEADER);
}

#[test]
fn tpdu_header_read() {
    let mut src = ReadCursor::new(&[
        0x03, 0x00, 0x00, 0x0c, // tpkt
        0x02, 0xf0, 0x80, // tpdu
        0x04, 0x01, 0x00, 0x01, 0x00, // payload
    ]);

    let tpkt = TpktHeader::read(&mut src).expect("tpkt");
    assert_eq!(tpkt.packet_length, 12);

    let tpdu = TpduHeader::read(&mut src, &tpkt).expect("tpdu");
    assert_eq!(tpdu.li, 2);
    assert_eq!(tpdu.code, TpduCode::DATA);
    assert_eq!(tpdu.fixed_part_size(), 3);
    assert_eq!(tpdu.variable_part_size(), 0);

    let payload_len = user_data_size(&tpkt, &tpdu);
    assert_eq!(payload_len, 5);
    assert_eq!(src.len(), payload_len);
}

#[test]
fn tpdu_header_write() {
    let expected = [
        0x02, 0xf0, 0x80, // data tpdu
    ];

    let mut buffer = [0; 3];
    let mut cursor = WriteCursor::new(&mut buffer);

    TpduHeader {
        li: 2,
        code: TpduCode::DATA,
    }
    .write(&mut cursor)
    .unwrap();

    assert_eq!(buffer, expected);
}

encode_decode_test! {
    nego_connection_request_rdp_security_without_cookie:
        X224(ConnectionRequest {
            nego_data: None,
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::empty(),
            correlation_info: None,
        }),
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x13, // length in BE
            // tpdu header
            0x0E, // length
            0xE0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class
            // variable part
            0x01, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, // RDP_NEG_REQ
        ];

    nego_connection_request_rdp_security_with_cookie:
        X224(ConnectionRequest {
            nego_data: Some(NegoRequestData::Cookie(Cookie("User".to_owned()))),
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::empty(),
            correlation_info: None,
        }),
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x2A, // length in BE
            // tpdu header
            0x25, // length
            0xE0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class
            // variable part
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
            0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
            0x01, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, // RDP_NEG_REQ
        ];

    nego_connection_request_ssl_security_with_cookie:
        X224(ConnectionRequest {
            nego_data: Some(NegoRequestData::Cookie(Cookie("User".to_owned()))),
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
            correlation_info: None,
        }),
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x2A, // length in BE
            // tpdu header
            0x25, // length
            0xE0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class
            // variable part
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
            0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
            0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00, // RDP_NEG_REQ
        ];

    nego_connection_request_ssl_security_with_flags:
        X224(ConnectionRequest {
            nego_data: Some(NegoRequestData::Cookie(Cookie("User".to_owned()))),
            flags: RequestFlags::RESTRICTED_ADMIN_MODE_REQUIRED | RequestFlags::REDIRECTED_AUTHENTICATION_MODE_REQUIRED,
            protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
            correlation_info: None,
        }),
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x2A, // length in BE
            // tpdu header
            0x25, // length
            0xE0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class
            // cookie
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
            0x73, 0x65, 0x72, 0x0D, 0x0A,
            // RDP_NEG_REQ
            0x01, // type
            0x03, // flags
            0x08, 0x00, // length
            0x03, 0x00, 0x00, 0x00, // request message
        ];

    nego_connection_request_with_correlation_info:
        X224(ConnectionRequest {
            nego_data: None,
            flags: RequestFlags::CORRELATION_INFO_PRESENT,
            protocol: SecurityProtocol::SSL,
            correlation_info: Some(CorrelationInfo {
                correlation_id: [0x01; 16],
            }),
        }),
        [
            // tpkt header
            0x03, 0x00, 0x00, 0x37,
            // tpdu header
            0x32, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x00,
            // RDP_NEG_REQ
            0x01, 0x08, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00,
            // RDP_NEG_CORRELATION_INFO
            0x06, 0x00, 0x24, 0x00,
            0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

    nego_connection_request_with_cookie_and_correlation_info:
        X224(ConnectionRequest {
            nego_data: Some(NegoRequestData::Cookie(Cookie("User".to_owned()))),
            flags: RequestFlags::CORRELATION_INFO_PRESENT,
            protocol: SecurityProtocol::SSL,
            correlation_info: Some(CorrelationInfo {
                correlation_id: [0x01; 16],
            }),
        }),
        [
            // tpkt header
            0x03, 0x00, 0x00, 0x4E,
            // tpdu header
            0x49, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x00,
            // cookie
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
            0x73, 0x65, 0x72, 0x0D, 0x0A,
            // RDP_NEG_REQ
            0x01, 0x08, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00,
            // RDP_NEG_CORRELATION_INFO
            0x06, 0x00, 0x24, 0x00,
            0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

    nego_confirm_response:
        X224(ConnectionConfirm::Response {
            flags: ResponseFlags::from_bits_retain(0x1F),
            protocol: SecurityProtocol::HYBRID,
        }),
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x13, // length in BE
            // tpdu header
            0x0E, // length
            0xD0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class
            // RDP_NEG_RSP
            0x02, // type
            0x1F, // flags
            0x08, 0x00, // length
            0x02, 0x00, 0x00, 0x00, // selected protocol
        ];

    nego_confirm_failure:
        X224(ConnectionConfirm::Failure {
            code: FailureCode::SSL_WITH_USER_AUTH_REQUIRED_BY_SERVER,
        }),
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x13, // length in BE
            // tpdu header
            0x0E,  // length
            0xD0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class
            // RDP_NEG_FAILURE
            0x03, // type
            0x00, // flags
            0x08, 0x00, // length
            0x06, 0x00, 0x00, 0x00, // failure code
        ];
}

#[test]
fn nego_connection_request_rejects_invalid_correlation_info() {
    let valid = ironrdp_core::encode_vec(&X224(ConnectionRequest {
        nego_data: None,
        flags: RequestFlags::CORRELATION_INFO_PRESENT,
        protocol: SecurityProtocol::SSL,
        correlation_info: Some(CorrelationInfo {
            correlation_id: [0x01; 16],
        }),
    }))
    .unwrap();

    // TPKT (4 bytes) + X.224 connection request header (7 bytes) +
    // RDP_NEG_REQ (8 bytes).
    const CORRELATION_INFO_OFFSET: usize = 19;

    let mut invalid_type = valid.clone();
    invalid_type[CORRELATION_INFO_OFFSET] = 0x05;
    assert!(ironrdp_core::decode::<X224<ConnectionRequest>>(&invalid_type).is_err());

    let mut invalid_flags = valid.clone();
    invalid_flags[CORRELATION_INFO_OFFSET + 1] = 0x01;
    assert!(ironrdp_core::decode::<X224<ConnectionRequest>>(&invalid_flags).is_err());

    let mut invalid_length = valid.clone();
    invalid_length[CORRELATION_INFO_OFFSET + 2] = 0x23;
    assert!(ironrdp_core::decode::<X224<ConnectionRequest>>(&invalid_length).is_err());

    let mut invalid_reserved = valid;
    invalid_reserved[CORRELATION_INFO_OFFSET + 20] = 0x01;
    assert!(ironrdp_core::decode::<X224<ConnectionRequest>>(&invalid_reserved).is_err());

    let mut oversized = ironrdp_core::encode_vec(&X224(ConnectionRequest {
        nego_data: None,
        flags: RequestFlags::CORRELATION_INFO_PRESENT,
        protocol: SecurityProtocol::SSL,
        correlation_info: Some(CorrelationInfo {
            correlation_id: [0x01; 16],
        }),
    }))
    .unwrap();
    oversized[3] += 1; // TPKT length
    oversized[4] += 1; // X.224 length indicator
    oversized.push(0);
    assert!(ironrdp_core::decode::<X224<ConnectionRequest>>(&oversized).is_err());
}

#[test]
fn nego_connection_request_derives_correlation_info_flag() {
    let correlation_info = CorrelationInfo {
        correlation_id: [0x01; 16],
    };

    let request_with_correlation_info = ConnectionRequest {
        nego_data: None,
        flags: RequestFlags::empty(),
        protocol: SecurityProtocol::SSL,
        correlation_info: Some(correlation_info),
    };
    let encoded = ironrdp_core::encode_vec(&X224(request_with_correlation_info)).unwrap();
    assert_eq!(encoded[12], RequestFlags::CORRELATION_INFO_PRESENT.bits());

    let request_without_correlation_info = ConnectionRequest {
        nego_data: None,
        flags: RequestFlags::CORRELATION_INFO_PRESENT | RequestFlags::RESTRICTED_ADMIN_MODE_REQUIRED,
        protocol: SecurityProtocol::SSL,
        correlation_info: None,
    };
    let encoded = ironrdp_core::encode_vec(&X224(request_without_correlation_info)).unwrap();
    assert_eq!(encoded[12], RequestFlags::RESTRICTED_ADMIN_MODE_REQUIRED.bits());
}

#[test]
fn nego_connection_request_rejects_unexpected_trailing_data() {
    let request = ConnectionRequest {
        nego_data: None,
        flags: RequestFlags::empty(),
        protocol: SecurityProtocol::SSL,
        correlation_info: None,
    };
    let mut encoded = ironrdp_core::encode_vec(&X224(request)).unwrap();
    encoded[3] += 1; // TPKT length
    encoded[4] += 1; // X.224 length indicator
    encoded.push(0);

    assert!(ironrdp_core::decode::<X224<ConnectionRequest>>(&encoded).is_err());
}

#[test]
fn nego_connection_request_rejects_truncated_negotiation_request() {
    const RDP_NEG_REQ_PREFIX: [u8; 7] = [0x01, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00];

    for truncated_size in 1..=RDP_NEG_REQ_PREFIX.len() {
        let mut payload = vec![
            // tpkt header
            0x03,
            0x00,
            0x00,
            u8::try_from(11 + truncated_size).expect("TPKT size fits in u8"),
            // tpdu header
            u8::try_from(6 + truncated_size).expect("TPDU size fits in u8"),
            0xE0,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
        ];
        payload.extend_from_slice(&RDP_NEG_REQ_PREFIX[..truncated_size]);

        assert!(ironrdp_core::decode::<X224<ConnectionRequest>>(&payload).is_err());
    }
}

#[test]
fn nego_request_unexpected_rdp_msg_type() {
    let payload = [
        // tpkt header
        0x03, // version
        0x00, // reserved
        0x00, 0x2A, // length in BE
        // tpdu header
        0x25, // length
        0xE0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class
        // variable part
        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
        0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
        // RDP message
        0x03, // type
        0x00, // flags
        0x08, 0x00, // length
        0x03, 0x00, 0x00, 0x00, // rest
    ];

    let e = ironrdp_core::decode::<X224<ConnectionRequest>>(&payload).err().unwrap();

    expect![[r#"
        Error {
            context: "Client X.224 Connection Request",
            kind: UnexpectedMessageType {
                got: 3,
                offset: Some(
                    35,
                ),
            },
            source: None,
        }
    "#]]
    .assert_debug_eq(&e);
}

#[test]
fn nego_confirm_unexpected_rdp_msg_type() {
    let payload = [
        // tpkt header
        0x03, // version
        0x00, // reserved
        0x00, 0x13, // length in BE
        // tpdu header
        0x0E, // length
        0xD0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class
        // RDP_NEG_REQ
        0xAF, // type
        0x1F, // flags
        0x08, 0x00, // length
        0x02, 0x00, 0x00, 0x00, // selected protocol
    ];

    let e = ironrdp_core::decode::<X224<ConnectionConfirm>>(&payload).err().unwrap();

    expect![[r#"
        Error {
            context: "Server X.224 Connection Confirm",
            kind: UnexpectedMessageType {
                got: 175,
                offset: Some(
                    12,
                ),
            },
            source: None,
        }
    "#]]
    .assert_debug_eq(&e);
}

#[test]
fn cookie_decode() {
    let payload = [
        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
        0x73, 0x65, 0x72, 0x0D, 0x0A, 0xFF, 0xFF,
    ];

    let cookie = Cookie::read(&mut ReadCursor::new(&payload))
        .expect("read cookie")
        .expect("cookie");

    assert_eq!(cookie.0, "User");
}

#[test]
fn routing_token_decode() {
    let payload = [
        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x3D, 0x33, 0x36, 0x34, 0x30, 0x32,
        0x30, 0x35, 0x32, 0x32, 0x38, 0x2E, 0x31, 0x35, 0x36, 0x32, 0x39, 0x2E, 0x30, 0x30, 0x30, 0x30, 0x0D, 0x0A,
        0xFF, 0xFF,
    ];

    let routing_token = RoutingToken::read(&mut ReadCursor::new(&payload))
        .expect("read routing token")
        .expect("routing token");

    assert_eq!(routing_token.0, "3640205228.15629.0000");
}

#[test]
fn raw_routing_token_roundtrip() {
    let token = OpaqueRoutingToken("tsv://MS Terminal Services Plugin.1.collection".to_owned());
    let mut buffer = vec![0; token.size()];
    token
        .write(&mut WriteCursor::new(&mut buffer))
        .expect("write raw routing token");
    assert_eq!(buffer, b"tsv://MS Terminal Services Plugin.1.collection\r\n");

    let decoded = OpaqueRoutingToken::read(&mut ReadCursor::new(&buffer))
        .expect("read raw routing token")
        .expect("raw routing token");
    assert_eq!(decoded, token);

    let request = ConnectionRequestWithOpaqueRoutingToken {
        request: ConnectionRequest {
            nego_data: None,
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::SSL,
            correlation_info: None,
        },
        routing_token: token,
    };
    let encoded = ironrdp_core::encode_vec(&X224(request.clone())).expect("encode connection request");
    let decoded = ironrdp_core::decode::<X224<ConnectionRequestWithOpaqueRoutingToken>>(&encoded)
        .expect("decode connection request");
    assert_eq!(decoded.0, request);

    let oversized = OpaqueRoutingToken("x".repeat(ironrdp_pdu::nego::MAX_ROUTING_TOKEN_LENGTH + 1));
    let mut buffer = vec![0; oversized.size()];
    assert!(oversized.write(&mut WriteCursor::new(&mut buffer)).is_err());
}

#[test]
fn not_a_cookie_decode() {
    let payload = [
        0x6e, 0x6f, 0x74, 0x20, 0x61, 0x20, 0x63, 0x6f, 0x6f, 0x6b, 0x69, 0x65, 0x0F, 0x42, 0x73, 0x65, 0x72, 0x0D,
        0x0A, 0xFF, 0xFF,
    ];

    let maybe_cookie = Cookie::read(&mut ReadCursor::new(&payload)).expect("read cookie");

    assert!(maybe_cookie.is_none());
}

#[test]
fn cookie_without_cr_lf_error_decode() {
    let payload = [
        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
        0x73, 0x65, 0x72,
    ];

    let e = Cookie::read(&mut ReadCursor::new(&payload)).err().unwrap();

    expect![[r#"
        Error {
            context: "Cookie",
            kind: NotEnoughBytes {
                received: 1,
                expected: 2,
                offset: Some(
                    20,
                ),
            },
            source: None,
        }
    "#]]
    .assert_debug_eq(&e);
}
