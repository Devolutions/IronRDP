use expect_test::{Expect, expect};
use ironrdp_rdcleanpath::{
    DNS_RESOLUTION_FAILED_ERROR_CODE, DetectionResult, EXPIRED_ERROR_CODE, GENERAL_ERROR_CODE, NEGOTIATION_ERROR_CODE,
    NOT_YET_VALID_ERROR_CODE, RDCleanPathErr, RDCleanPathMessage, RDCleanPathPdu, REVOKED_ERROR_CODE,
    TARGET_UNREACHABLE_ERROR_CODE, TLS_HANDSHAKE_FAILED_ERROR_CODE, VERSION_1,
};
use rstest::rstest;

fn request() -> RDCleanPathPdu {
    RDCleanPathPdu::new_request(
        vec![0xDE, 0xAD, 0xBE, 0xFF],
        "destination".to_owned(),
        "proxy auth".to_owned(),
        Some("PCB".to_owned()),
    )
    .unwrap()
}

const REQUEST_DER: &[u8] = &[
    0x30, 0x32, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA2, 0xD, 0xC, 0xB, 0x64, 0x65, 0x73, 0x74, 0x69, 0x6E, 0x61, 0x74,
    0x69, 0x6F, 0x6E, 0xA3, 0xC, 0xC, 0xA, 0x70, 0x72, 0x6F, 0x78, 0x79, 0x20, 0x61, 0x75, 0x74, 0x68, 0xA5, 0x5, 0xC,
    0x3, 0x50, 0x43, 0x42, 0xA6, 0x6, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF,
];

fn vmconnect_request() -> RDCleanPathPdu {
    RDCleanPathPdu::new_vmconnect_request("destination".to_owned(), "proxy auth".to_owned(), "PCB".to_owned()).unwrap()
}

const VMCONNECT_REQUEST_DER: &[u8] = &[
    0x30, 0x2A, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA2, 0xD, 0xC, 0xB, 0x64, 0x65, 0x73, 0x74, 0x69, 0x6E, 0x61, 0x74,
    0x69, 0x6F, 0x6E, 0xA3, 0xC, 0xC, 0xA, 0x70, 0x72, 0x6F, 0x78, 0x79, 0x20, 0x61, 0x75, 0x74, 0x68, 0xA5, 0x5, 0xC,
    0x3, 0x50, 0x43, 0x42,
];

fn response_success() -> RDCleanPathPdu {
    RDCleanPathPdu::new_response(
        "192.168.7.95".to_owned(),
        vec![0xDE, 0xAD, 0xBE, 0xFF],
        [
            vec![0xDE, 0xAD, 0xBE, 0xFF],
            vec![0xDE, 0xAD, 0xBE, 0xFF],
            vec![0xDE, 0xAD, 0xBE, 0xFF],
        ],
    )
    .unwrap()
}

const RESPONSE_SUCCESS_DER: &[u8] = &[
    0x30, 0x34, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA6, 0x6, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF, 0xA7, 0x14, 0x30, 0x12,
    0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF, 0xA9, 0xE,
    0xC, 0xC, 0x31, 0x39, 0x32, 0x2E, 0x31, 0x36, 0x38, 0x2E, 0x37, 0x2E, 0x39, 0x35,
];

fn vmconnect_response() -> RDCleanPathPdu {
    RDCleanPathPdu::new_vmconnect_response(
        "192.168.7.95".to_owned(),
        [
            vec![0xDE, 0xAD, 0xBE, 0xFF],
            vec![0xDE, 0xAD, 0xBE, 0xFF],
            vec![0xDE, 0xAD, 0xBE, 0xFF],
        ],
    )
    .unwrap()
}

const VMCONNECT_RESPONSE_DER: &[u8] = &[
    0x30, 0x2C, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA7, 0x14, 0x30, 0x12, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF, 0x4, 0x4,
    0xDE, 0xAD, 0xBE, 0xFF, 0x4, 0x4, 0xDE, 0xAD, 0xBE, 0xFF, 0xA9, 0xE, 0xC, 0xC, 0x31, 0x39, 0x32, 0x2E, 0x31, 0x36,
    0x38, 0x2E, 0x37, 0x2E, 0x39, 0x35,
];

fn response_http_error() -> RDCleanPathPdu {
    RDCleanPathPdu::new_http_error(500)
}

const RESPONSE_HTTP_ERROR_DER: &[u8] = &[
    0x30, 0x15, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA1, 0xD, 0x30, 0xB, 0xA0, 0x3, 0x2, 0x1, 0x1, 0xA1, 0x4, 0x2, 0x2,
    0x1, 0xF4,
];

fn response_tls_error() -> RDCleanPathPdu {
    RDCleanPathPdu::new_tls_error(48)
}

const RESPONSE_TLS_ERROR_DER: &[u8] = &[
    0x30, 0x14, 0xA0, 0x04, 0x02, 0x02, 0x0D, 0x3E, 0xA1, 0x0C, 0x30, 0x0A, 0xA0, 0x03, 0x02, 0x01, 0x01, 0xA3, 0x03,
    0x02, 0x01, 0x30,
];

#[rstest]
#[case(request())]
#[case(vmconnect_request())]
#[case(response_success())]
#[case(vmconnect_response())]
#[case(response_http_error())]
#[case(response_tls_error())]
fn smoke(#[case] message: RDCleanPathPdu) {
    let encoded = message.to_der().unwrap();
    let decoded = RDCleanPathPdu::from_der(&encoded).unwrap();
    assert_eq!(message, decoded);
}

macro_rules! assert_serialization {
    ($left:expr, $right:expr) => {{
        if $left != $right {
            let left = hex::encode(&$left);
            let right = hex::encode(&$right);
            let comparison = pretty_assertions::StrComparison::new(&left, &right);
            panic!(
                "assertion failed: `({} == {})`\n\n{comparison}",
                stringify!($left),
                stringify!($right),
            );
        }
    }};
}

#[rstest]
#[case(request(), REQUEST_DER)]
#[case(vmconnect_request(), VMCONNECT_REQUEST_DER)]
#[case(response_success(), RESPONSE_SUCCESS_DER)]
#[case(vmconnect_response(), VMCONNECT_RESPONSE_DER)]
#[case(response_http_error(), RESPONSE_HTTP_ERROR_DER)]
#[case(response_tls_error(), RESPONSE_TLS_ERROR_DER)]
fn serialization(#[case] message: RDCleanPathPdu, #[case] expected_der: &[u8]) {
    let encoded = message.to_der().unwrap();
    assert_serialization!(encoded, expected_der);
}

#[rstest]
#[case(REQUEST_DER)]
#[case(VMCONNECT_REQUEST_DER)]
#[case(RESPONSE_SUCCESS_DER)]
#[case(VMCONNECT_RESPONSE_DER)]
#[case(RESPONSE_HTTP_ERROR_DER)]
#[case(RESPONSE_TLS_ERROR_DER)]
fn detect(#[case] der: &[u8]) {
    let result = RDCleanPathPdu::detect(der);

    let DetectionResult::Detected {
        version: detected_version,
        total_length: detected_length,
    } = result
    else {
        panic!("unexpected result: {result:?}");
    };

    assert_eq!(detected_version, VERSION_1);
    assert_eq!(detected_length, der.len());
}

#[rstest]
#[case(&[])]
#[case(&[0x30])]
#[case(&[0x30, 0x15])]
#[case(&[0x30, 0x15, 0xA0])]
#[case(&[0x30, 0x32, 0xA0, 0x4])]
#[case(&[0x30, 0x32, 0xA0, 0x4, 0x2])]
#[case(&[0x30, 0x32, 0xA0, 0x4, 0x2, 0x2])]
#[case(&[0x30, 0x32, 0xA0, 0x4, 0x2, 0x2, 0xD])]
fn detect_not_enough(#[case] payload: &[u8]) {
    let result = RDCleanPathPdu::detect(payload);
    assert_eq!(result, DetectionResult::NotEnoughBytes);
}

#[test]
fn vmconnect_payload_roundtrips_as_unicode_instead_of_binary_pcb() {
    let payload = format!("{};EnhancedMode=1;名字=虚拟机", "a".repeat(64));
    let pdu = RDCleanPathPdu::new_vmconnect_request("destination".to_owned(), "proxy auth".to_owned(), payload.clone());
    let pdu = pdu.unwrap();

    let decoded = RDCleanPathPdu::from_der(&pdu.to_der().unwrap())
        .unwrap()
        .into_message()
        .unwrap();

    assert!(matches!(
        decoded,
        RDCleanPathMessage::VmConnectRequest { pcb_payload, .. } if pcb_payload == payload
    ));
}

#[test]
fn ordinary_request_keeps_generic_pcb_with_x224() {
    let decoded = request().into_message().unwrap();

    assert!(matches!(
        decoded,
        RDCleanPathMessage::Request {
            preconnection_blob: Some(pcb),
            ..
        } if pcb == "PCB"
    ));
}

#[test]
fn request_without_x224_or_vmconnect_payload_is_rejected() {
    let pdu = RDCleanPathPdu {
        destination: Some("destination".to_owned()),
        proxy_auth: Some("proxy auth".to_owned()),
        ..RDCleanPathPdu::default()
    };

    assert!(pdu.into_enum().is_err());
}

#[test]
fn vmconnect_request_rejects_empty_payload() {
    assert!(
        RDCleanPathPdu::new_vmconnect_request("destination".to_owned(), "proxy auth".to_owned(), String::new(),)
            .is_err()
    );

    assert!(
        RDCleanPathMessage::VmConnectRequest {
            destination: "destination".to_owned(),
            proxy_auth: "proxy auth".to_owned(),
            server_auth: None,
            pcb_payload: "   ".to_owned(),
        }
        .into_pdu()
        .is_err()
    );
}

#[test]
fn response_shape_selects_explicit_semantic_variant() {
    assert!(matches!(
        response_success().into_message().unwrap(),
        RDCleanPathMessage::Response { .. }
    ));
    assert!(matches!(
        vmconnect_response().into_message().unwrap(),
        RDCleanPathMessage::VmConnectResponse { .. }
    ));
}

#[rstest]
#[case::http(
    RDCleanPathErr {
        error_code: GENERAL_ERROR_CODE,
        http_status_code: Some(404),
        wsa_last_error: None,
        tls_alert_code: None,
    },
    expect!["general error (code 1); HTTP 404 not found"],
)]
#[case::wsa(
    RDCleanPathErr {
        error_code: GENERAL_ERROR_CODE,
        http_status_code: None,
        wsa_last_error: Some(10061),
        tls_alert_code: None,
    },
    expect!["general error (code 1); WSA 10061 connection refused"],
)]
#[case::tls(
    RDCleanPathErr {
        error_code: GENERAL_ERROR_CODE,
        http_status_code: None,
        wsa_last_error: None,
        tls_alert_code: Some(40),
    },
    expect!["general error (code 1); TLS alert 40 handshake failure"],
)]
#[case::nego(
    RDCleanPathErr {
        error_code: NEGOTIATION_ERROR_CODE,
        http_status_code: None,
        wsa_last_error: None,
        tls_alert_code: None,
    },
    expect!["negotiation error (code 2)"],
)]
#[case::combined(
    RDCleanPathErr {
        error_code: GENERAL_ERROR_CODE,
        http_status_code: Some(502),
        wsa_last_error: Some(10060),
        tls_alert_code: Some(45),
    },
    expect!["general error (code 1); HTTP 502 bad gateway; WSA 10060 connection timed out; TLS alert 45 certificate expired"],
)]
#[case::unknown_codes(
    RDCleanPathErr {
        error_code: 99,
        http_status_code: Some(999),
        wsa_last_error: Some(65000),
        tls_alert_code: Some(255),
    },
    expect!["unknown error (code 99); HTTP 999 unknown HTTP status; WSA 65000 unknown WSA error; TLS alert 255 unknown TLS alert"],
)]
fn error_display(#[case] error: RDCleanPathErr, #[case] expected: Expect) {
    expected.assert_eq(&error.to_string());
}

fn error_with_code(error_code: u16) -> RDCleanPathErr {
    RDCleanPathErr {
        error_code,
        http_status_code: None,
        wsa_last_error: None,
        tls_alert_code: None,
    }
}

#[test]
fn standard_error_code_display() {
    let descriptions = [
        EXPIRED_ERROR_CODE,
        NOT_YET_VALID_ERROR_CODE,
        REVOKED_ERROR_CODE,
        TARGET_UNREACHABLE_ERROR_CODE,
        DNS_RESOLUTION_FAILED_ERROR_CODE,
        TLS_HANDSHAKE_FAILED_ERROR_CODE,
    ]
    .map(|error_code| error_with_code(error_code).to_string())
    .join("\n");

    // The codes are a wire contract with deployed peers: never change them.
    expect![["
        credential expired (code 100)
        credential not yet valid (code 101)
        credential revoked (code 102)
        target unreachable (code 103)
        DNS resolution failed (code 104)
        TLS handshake failed (code 105)"]]
    .assert_eq(&descriptions);
}

#[test]
fn unknown_error_code_is_kept_as_general_error() {
    // The crate does not know what an implementation-specific code means.
    let error = error_with_code(16384);
    let pdu = RDCleanPathPdu {
        error: Some(error.clone()),
        ..RDCleanPathPdu::default()
    };

    assert_eq!(pdu.into_message(), Ok(RDCleanPathMessage::GeneralErr(error)));
}

#[test]
fn implementation_specific_error_code_display() {
    let descriptions = [16383, 16384, 32767, 32768]
        .map(|error_code| error_with_code(error_code).to_string())
        .join("\n");

    expect![["
        unknown error (code 16383)
        implementation-specific error (code 16384)
        implementation-specific error (code 32767)
        unknown error (code 32768)"]]
    .assert_eq(&descriptions);
}
