use expect_test::{expect, Expect};
use ironrdp_rdcleanpath::{
    DetectionResult, RDCleanPathErr, RDCleanPathPdu, GENERAL_ERROR_CODE, NEGOTIATION_ERROR_CODE, VERSION_1,
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
#[case(response_success())]
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
#[case(response_success(), RESPONSE_SUCCESS_DER)]
#[case(response_http_error(), RESPONSE_HTTP_ERROR_DER)]
#[case(response_tls_error(), RESPONSE_TLS_ERROR_DER)]
fn serialization(#[case] message: RDCleanPathPdu, #[case] expected_der: &[u8]) {
    let encoded = message.to_der().unwrap();
    assert_serialization!(encoded, expected_der);
}

#[rstest]
#[case(REQUEST_DER)]
#[case(RESPONSE_SUCCESS_DER)]
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
