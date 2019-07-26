use super::*;

#[test]
fn rdp_negotiation_data_is_written_to_request_if_nla_security() {
    let mut buffer = Vec::new();
    let expected = [0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00];

    let request = Request {
        nego_data: Some(NegoData::Cookie("a".to_string())),
        flags: RequestFlags::empty(),
        protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
        src_ref: 0,
    };

    request.to_buffer(&mut buffer).unwrap();

    assert_eq!(
        buffer[buffer.len() - usize::from(RDP_NEG_DATA_LENGTH)..],
        expected
    );
}

#[test]
fn rdp_negotiation_data_is_not_written_if_rdp_security() {
    #[rustfmt::skip]
    let expected = [
        // tpkt header
        0x3u8, // version
        0x0, // reserved
        0x00, 0x22, // lenght in BE

        // tpdu
        0x6, // length
        0xe0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
        0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A,
    ];
    let mut buff = Vec::new();

    let request = Request {
        nego_data: Some(NegoData::Cookie("User".to_string())),
        flags: RequestFlags::empty(),
        protocol: SecurityProtocol::RDP,
        src_ref: 0,
    };

    request.to_buffer(&mut buff).unwrap();

    assert_eq!(expected.as_ref(), buff.as_slice());
}

#[test]
fn negotiation_request_is_written_correclty() {
    #[rustfmt::skip]
    let expected = [
        // tpkt header
        0x3u8, // version
        0x0, // reserved
        0x00, 0x2a, // lenght in BE

        // tpdu
        0x6, // length
        0xe0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
        0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A, 0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00,
        0x00,
    ];
    let mut buff = Vec::new();

    let request = Request {
        nego_data: Some(NegoData::Cookie("User".to_string())),
        flags: RequestFlags::empty(),
        protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
        src_ref: 0,
    };

    request.to_buffer(&mut buff).unwrap();

    assert_eq!(expected.as_ref(), buff.as_slice());
}

#[test]
fn negotiation_response_is_processed_correctly() {
    let expected_flags = ResponseFlags::all();

    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x13, // lenght in BE

        // tpdu
        0x6, // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x02, // negotiation message
        expected_flags.bits(),
        0x08, 0x00, // length
        0x02, 0x00, 0x00, 0x00, // selected protocol
    ];

    let response_data = Some(ResponseData::Response {
        flags: expected_flags,
        protocol: SecurityProtocol::HYBRID,
    });

    let response = Response {
        response: response_data,
        dst_ref: 0,
        src_ref: 0,
    };

    assert_eq!(response, Response::from_buffer(buffer.as_ref()).unwrap());
}

#[test]
fn wrong_message_code_in_negotiation_response_results_in_error() {
    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x13, // lenght in BE

        // tpdu
        0x6,  // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0xAF, // negotiation message
        0x1F, // flags
        0x08, 0x00, // length
        0x02, 0x00, 0x00, 0x00, // selected protocol
    ];

    match Response::from_buffer(buffer.as_ref()) {
        Err(NegotiationError::IOError(ref e)) if e.kind() == io::ErrorKind::InvalidData => (),
        Err(_e) => panic!("wrong error type"),
        _ => panic!("error expected"),
    }
}

#[test]
fn negotiation_failure_in_response_results_in_error() {
    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x13, // lenght in BE

        // tpdu
        0x6,  // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x03, // negotiation message
        0x1F, // flags
        0x08, 0x00, // length
        0x06, 0x00, 0x00, 0x00, // failure code
    ];

    match Response::from_buffer(buffer.as_ref()) {
        Err(NegotiationError::ResponseFailure(e))
            if e == FailureCode::SSLWithUserAuthRequiredByServer => {}
        Err(_e) => panic!("wrong error type"),
        _ => panic!("error expected"),
    }
}

#[test]
fn cookie_in_request_is_parsed_correctly() {
    let request = [
        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
        0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A, 0xFF, 0xFF,
    ];

    let (nego_data, _read_len) = read_nego_data(request.as_ref()).unwrap();

    match nego_data {
        NegoData::Cookie(cookie) => assert_eq!(cookie, "User"),
        _ => panic!("Cookie expected"),
    };
}

#[test]
fn routing_token_in_request_is_parsed_correctly() {
    let request = [
        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x3D, 0x33, 0x36,
        0x34, 0x30, 0x32, 0x30, 0x35, 0x32, 0x32, 0x38, 0x2E, 0x31, 0x35, 0x36, 0x32, 0x39, 0x2E,
        0x30, 0x30, 0x30, 0x30, 0x0D, 0x0A, 0xFF, 0xFF,
    ];

    let (nego_data, _read_len) = read_nego_data(request.as_ref()).unwrap();

    match nego_data {
        NegoData::RoutingToken(routing_token) => assert_eq!(routing_token, "3640205228.15629.0000"),
        _ => panic!("Routing token expected"),
    };
}

#[test]
fn read_string_with_cr_lf_on_non_value_results_in_error() {
    let request = [
        0x6e, 0x6f, 0x74, 0x20, 0x61, 0x20, 0x63, 0x6f, 0x6f, 0x6b, 0x69, 0x65, 0x0F, 0x42, 0x73,
        0x65, 0x72, 0x0D, 0x0A, 0xFF, 0xFF,
    ];

    match read_string_with_cr_lf(&mut request.as_ref(), COOKIE_PREFIX) {
        Err(ref e) if e.kind() == io::ErrorKind::InvalidData => (),
        Err(_e) => panic!("wrong error type"),
        _ => panic!("error expected"),
    }
}

#[test]
fn read_string_with_cr_lf_on_unterminated_message_results_in_error() {
    let request = [
        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
        0x68, 0x3D, 0x55, 0x73, 0x65, 0x72,
    ];

    match read_string_with_cr_lf(&mut request.as_ref(), COOKIE_PREFIX) {
        Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => (),
        Err(_e) => panic!("wrong error type"),
        _ => panic!("error expected"),
    }
}

#[test]
fn read_string_with_cr_lf_on_unterminated_with_cr_message() {
    let request = [
        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
        0x68, 0x3D, 0x0a,
    ];

    match read_string_with_cr_lf(&mut request.as_ref(), COOKIE_PREFIX) {
        Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => (),
        Err(_e) => panic!("wrong error type"),
        _ => panic!("error expected"),
    }
}

#[test]
fn negotiation_request_with_negotiation_data_is_parsed_correctly() {
    let expected_flags = RequestFlags::RESTRICTED_ADMIN_MODE_REQUIRED
        | RequestFlags::REDIRECTED_AUTHENTICATION_MODE_REQUIRED;

    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x2a, // lenght in BE

        // tpdu
        0x6, // length
        0xe0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
        0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
        0x01, // request code
        expected_flags.bits(),
        0x08, 0x00, 0x03, 0x00, 0x00, 0x00, // request message
    ];

    let request = Request {
        nego_data: Some(NegoData::Cookie("User".to_string())),
        flags: expected_flags,
        protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
        src_ref: 0,
    };

    assert_eq!(request, Request::from_buffer(buffer.as_ref()).unwrap());
}

#[test]
fn negotiation_request_without_variable_fields_is_parsed_correctly() {
    let expected_flags = RequestFlags::RESTRICTED_ADMIN_MODE_REQUIRED
        | RequestFlags::REDIRECTED_AUTHENTICATION_MODE_REQUIRED;

    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x13, // lenght in BE
        // tpdu
        0x6, // length
        0xe0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x01, // request code
        expected_flags.bits(),
        0x08, 0x00, 0x03, 0x00, 0x00, 0x00, // request message
    ];

    let request = Request {
        nego_data: None,
        flags: expected_flags,
        protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
        src_ref: 0,
    };

    assert_eq!(request, Request::from_buffer(buffer.as_ref()).unwrap());
}

#[test]
fn negotiation_request_without_negotiation_data_is_parsed_correctly() {
    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x22, // lenght in BE
        // tpdu
        0x6,  // length
        0xe0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
        0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
    ];

    let request = Request {
        nego_data: Some(NegoData::Cookie("User".to_string())),
        flags: RequestFlags::empty(),
        protocol: SecurityProtocol::RDP,
        src_ref: 0,
    };

    assert_eq!(request, Request::from_buffer(buffer.as_ref()).unwrap());
}

#[test]
fn negotiation_request_with_invalid_negotiation_code_results_in_error() {
    #[rustfmt::skip]
    let request = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x2a, // lenght in BE
        // tpdu
        0x6,  // length
        0xe0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
        0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
        0x03, // request code
        0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00, // request message
    ];

    match Request::from_buffer(request.as_ref()) {
        Err(NegotiationError::IOError(ref e)) if e.kind() == io::ErrorKind::InvalidData => (),
        Err(_e) => panic!("wrong error type"),
        _ => panic!("error expected"),
    }
}

#[test]
fn negotiation_response_is_written_correctly() {
    let flags = ResponseFlags::all();

    #[rustfmt::skip]
    let expected = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x13, // lenght in BE

        // tpdu
        0x6, // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x02, // negotiation message
        flags.bits(),
        0x08, 0x00, // length
        0x02, 0x00, 0x00, 0x00, // selected protocol
    ];

    let mut buffer = Vec::new();

    let response_data = Some(ResponseData::Response {
        flags,
        protocol: SecurityProtocol::HYBRID,
    });

    let response = Response {
        response: response_data,
        dst_ref: 0,
        src_ref: 0,
    };

    response.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, expected);
}

#[test]
fn negotiation_error_is_written_correclty() {
    #[rustfmt::skip]
    let expected = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x13, // lenght in BE

        // tpdu
        0x6, // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x03, // negotiation message
        0x00,
        0x08, 0x00, // length
        0x02, 0x00, 0x00, 0x00, // error code
    ];

    let mut buffer = Vec::new();

    let failure_data = Some(ResponseData::Failure {
        code: FailureCode::SSLNotAllowedByServer,
    });

    let response = Response {
        response: failure_data,
        dst_ref: 0,
        src_ref: 0,
    };

    response.to_buffer(&mut buffer).unwrap();

    assert_eq!(buffer, expected);
}

#[test]
fn buffer_length_is_correct_for_negatiation_request() {
    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x2a, // lenght in BE

        // tpdu
        0x6,  // length
        0xe0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
        0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A, 0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00,
        0x00,
    ];

    let request = Request {
        nego_data: Some(NegoData::Cookie("User".to_string())),
        flags: RequestFlags::empty(),
        protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
        src_ref: 0,
    };

    assert_eq!(request.buffer_length(), buffer.len());
}

#[test]
fn buffer_length_is_correct_for_negotiation_response() {
    let flags = ResponseFlags::all();
    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00,
        0x13, // lenght in BE

        // tpdu
        0x6,  // length
        0xd0, // code
        0x00,
        0x00, // dst_ref
        0x00,
        0x00, // src_ref
        0x00, // class

        0x02, // negotiation message
        flags.bits(),
        0x08,
        0x00, // length
        0x02,
        0x00,
        0x00,
        0x00, // selected protocol
    ];

    let response_data = Some(ResponseData::Response {
        flags,
        protocol: SecurityProtocol::HYBRID,
    });

    let response = Response {
        response: response_data,
        dst_ref: 0,
        src_ref: 0,
    };

    assert_eq!(response.buffer_length(), buffer.len());
}

#[test]
fn from_buffer_correctly_parses_negotiation_failure() {
    #[rustfmt::skip]
    let expected = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x13, // lenght in BE

        // tpdu
        0x6, // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x03, // negotiation message
        0x00,
        0x08, 0x00, // length
        0x02, 0x00, 0x00, 0x00, // error code
    ];

    match Response::from_buffer(expected.as_ref()) {
        Err(NegotiationError::ResponseFailure(_)) => (),
        Err(_e) => panic!("invalid error type"),
        Ok(_) => panic!("error expected"),
    }
}

#[test]
fn buffer_length_is_correct_for_negotiation_failure() {
    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x13, // lenght in BE

        // tpdu
        0x6,  // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class

        0x03, // negotiation message
        0x00, 0x08, 0x00, // length
        0x02, 0x00, 0x00, 0x00, // error code
    ];

    let failure_data = Some(ResponseData::Failure {
        code: FailureCode::SSLNotAllowedByServer,
    });

    let failure = Response {
        response: failure_data,
        dst_ref: 0,
        src_ref: 0,
    };

    assert_eq!(buffer.len(), failure.buffer_length());
}

#[test]
fn read_and_check_tpdu_header_reads_invalid_data_correctly() {
    let buffer = [
        0x6,  // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class
    ];

    assert!(crate::x224::read_and_check_tpdu_header(buffer.as_ref(), X224TPDUType::Data).is_err());
}

#[test]
fn read_and_check_tpdu_header_reads_correct_data_correctly() {
    let buffer = [
        0x6,  // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x00, // class
    ];

    crate::x224::read_and_check_tpdu_header(buffer.as_ref(), X224TPDUType::ConnectionConfirm)
        .unwrap();
}

#[test]
fn invalid_class_is_handeled_correctly() {
    #[rustfmt::skip]
    let buffer = [
        // tpkt header
        0x3, // version
        0x0, // reserved
        0x00, 0x13, // lenght in BE

        // tpdu
        0x6, // length
        0xd0, // code
        0x00, 0x00, // dst_ref
        0x00, 0x00, // src_ref
        0x01, // class

        0x03, // negotiation message
        0x00,
        0x08, 0x00, // length
        0x02, 0x00, 0x00, 0x00, // error code
    ];

    assert!(Response::from_buffer(buffer.as_ref()).is_err());
}
