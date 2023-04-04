use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::nego::{
    ConnectionConfirm, ConnectionRequest, Cookie, FailureCode, NegoRequestData, RequestFlags, ResponseFlags,
    RoutingToken, SecurityProtocol,
};
use ironrdp_pdu::tpdu::{TpduCode, TpduHeader};
use ironrdp_pdu::tpkt::TpktHeader;
use ironrdp_pdu::x224::user_data_size;
use ironrdp_pdu::Error;
use ironrdp_pdu_samples::encode_decode_test;

const SAMPLE_TPKT_HEADER_BINARY: [u8; 4] = [
    0x3, // version
    0x0, // reserved
    0x5, 0x42, // lenght in BE
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
    nego_connection_request_rdp_security_with_cookie:
        ConnectionRequest {
            nego_data: Some(NegoRequestData::Cookie(Cookie("User".to_owned()))),
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::RDP,
        },
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x22, // lenght in BE
            // tpdu header
            0x1D, // length
            0xE0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class
            // variable part
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
            0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
        ];

    nego_connection_request_ssl_security_with_cookie:
        ConnectionRequest {
            nego_data: Some(NegoRequestData::Cookie(Cookie("User".to_owned()))),
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
        },
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x2A, // lenght in BE
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
        ConnectionRequest {
            nego_data: Some(NegoRequestData::Cookie(Cookie("User".to_owned()))),
            flags: RequestFlags::RESTRICTED_ADMIN_MODE_REQUIRED | RequestFlags::REDIRECTED_AUTHENTICATION_MODE_REQUIRED,
            protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
        },
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x2A, // lenght in BE
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

    nego_confirm_response:
        ConnectionConfirm::Response {
            flags: ResponseFlags::from_bits_truncate(0x1F),
            protocol: SecurityProtocol::HYBRID,
        },
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x13, // lenght in BE
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
        ConnectionConfirm::Failure {
            code: FailureCode::SSL_WITH_USER_AUTH_REQUIRED_BY_SERVER,
        },
        [
            // tpkt header
            0x03, // version
            0x00, // reserved
            0x00, 0x13, // lenght in BE
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
fn nego_request_unexpected_rdp_msg_type() {
    let payload = [
        // tpkt header
        0x03, // version
        0x00, // reserved
        0x00, 0x2A, // lenght in BE
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

    let e = ironrdp_pdu::decode::<ConnectionRequest>(&payload).err().unwrap();

    if let Error::UnexpectedMessageType { name, got } = e {
        assert_eq!(name, "Client X.224 Connection Request");
        assert_eq!(got, 0x03);
    } else {
        panic!("unexpected error: {e}");
    }
}

#[test]
fn nego_confirm_unexpected_rdp_msg_type() {
    let payload = [
        // tpkt header
        0x03, // version
        0x00, // reserved
        0x00, 0x13, // lenght in BE
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

    let e = ironrdp_pdu::decode::<ConnectionConfirm>(&payload).err().unwrap();

    if let Error::UnexpectedMessageType { name, got } = e {
        assert_eq!(name, "Server X.224 Connection Confirm");
        assert_eq!(got, 0xAF);
    } else {
        panic!("unexpected error: {e}");
    }
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

    if let Error::NotEnoughBytes {
        name,
        received,
        expected,
    } = e
    {
        assert_eq!(name, "Cookie");
        assert_eq!(received, 1);
        assert_eq!(expected, 2);
    } else {
        panic!("unexpected error: {e}");
    }
}
