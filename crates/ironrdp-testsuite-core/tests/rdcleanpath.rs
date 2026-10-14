use expect_test::{Expect, expect};
use ironrdp_rdcleanpath::{
    AGENT_UNAVAILABLE_ERROR_CODE, BAD_SIGNATURE_ERROR_CODE, DNS_RESOLUTION_FAILED_ERROR_CODE, DetectionResult,
    EXPIRED_ERROR_CODE, GATEWAY_ID_MISMATCH_ERROR_CODE, GENERAL_ERROR_CODE, NEGOTIATION_ERROR_CODE,
    NOT_YET_VALID_ERROR_CODE, RDCleanPathErr, RDCleanPathMessage, RDCleanPathPdu, REVOKED_ERROR_CODE,
    TARGET_UNREACHABLE_ERROR_CODE, TLS_HANDSHAKE_FAILED_ERROR_CODE, UNEXPECTED_REPLAY_ERROR_CODE, VERSION_1,
    WRONG_TOKEN_TYPE_ERROR_CODE,
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

fn response_reason_error() -> RDCleanPathPdu {
    RDCleanPathPdu {
        error: Some(RDCleanPathErr {
            error_code: EXPIRED_ERROR_CODE,
            http_status_code: Some(401),
            wsa_last_error: None,
            tls_alert_code: None,
        }),
        ..RDCleanPathPdu::default()
    }
}

// Same layout as RESPONSE_HTTP_ERROR_DER: only the error code (0x65) and HTTP status (0x0191) values differ.
const RESPONSE_REASON_ERROR_DER: &[u8] = &[
    0x30, 0x15, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA1, 0xD, 0x30, 0xB, 0xA0, 0x3, 0x2, 0x1, 0x65, 0xA1, 0x4, 0x2, 0x2,
    0x1, 0x91,
];

#[rstest]
#[case(request())]
#[case(vmconnect_request())]
#[case(response_success())]
#[case(vmconnect_response())]
#[case(response_http_error())]
#[case(response_tls_error())]
#[case(response_reason_error())]
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
#[case(response_reason_error(), RESPONSE_REASON_ERROR_DER)]
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
#[case(RESPONSE_REASON_ERROR_DER)]
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
#[case::reason_http(
    RDCleanPathErr {
        error_code: EXPIRED_ERROR_CODE,
        http_status_code: Some(401),
        wsa_last_error: None,
        tls_alert_code: None,
    },
    expect!["token expired (code 101); HTTP 401 unauthorized"],
)]
#[case::reason_wsa(
    RDCleanPathErr {
        error_code: TARGET_UNREACHABLE_ERROR_CODE,
        http_status_code: None,
        wsa_last_error: Some(10061),
        tls_alert_code: None,
    },
    expect!["target unreachable (code 107); WSA 10061 connection refused"],
)]
#[case::reason_tls(
    RDCleanPathErr {
        error_code: TLS_HANDSHAKE_FAILED_ERROR_CODE,
        http_status_code: None,
        wsa_last_error: None,
        tls_alert_code: Some(40),
    },
    expect!["TLS handshake failed (code 109); TLS alert 40 handshake failure"],
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

/// Reason error codes, their wire values, and their names.
///
/// The wire values are a contract with deployed peers: never change them.
const REASON_ERROR_CODES: &[(u16, u16, &str)] = &[
    (BAD_SIGNATURE_ERROR_CODE, 100, "bad_signature"),
    (EXPIRED_ERROR_CODE, 101, "expired"),
    (NOT_YET_VALID_ERROR_CODE, 102, "not_yet_valid"),
    (GATEWAY_ID_MISMATCH_ERROR_CODE, 103, "gateway_id_mismatch"),
    (REVOKED_ERROR_CODE, 104, "revoked"),
    (UNEXPECTED_REPLAY_ERROR_CODE, 105, "unexpected_replay"),
    (WRONG_TOKEN_TYPE_ERROR_CODE, 106, "wrong_token_type"),
    (TARGET_UNREACHABLE_ERROR_CODE, 107, "target_unreachable"),
    (DNS_RESOLUTION_FAILED_ERROR_CODE, 108, "dns_resolution_failed"),
    (TLS_HANDSHAKE_FAILED_ERROR_CODE, 109, "tls_handshake_failed"),
    (AGENT_UNAVAILABLE_ERROR_CODE, 110, "agent_unavailable"),
];

fn error_with_code(error_code: u16) -> RDCleanPathErr {
    RDCleanPathErr {
        error_code,
        http_status_code: None,
        wsa_last_error: None,
        tls_alert_code: None,
    }
}

#[test]
fn reason_error_codes_are_stable() {
    for &(error_code, wire_value, reason) in REASON_ERROR_CODES {
        assert_eq!(error_code, wire_value, "{reason}");
        assert_eq!(error_with_code(error_code).reason(), Some(reason));
        assert_eq!(RDCleanPathErr::error_code_for_reason(reason), Some(error_code));
    }
}

#[test]
fn reason_mapping_covers_only_listed_codes() {
    let reasons: Vec<(u16, &str)> = (0..=u16::MAX)
        .filter_map(|error_code| Some((error_code, error_with_code(error_code).reason()?)))
        .collect();

    let expected: Vec<(u16, &str)> = REASON_ERROR_CODES
        .iter()
        .map(|&(error_code, _, reason)| (error_code, reason))
        .collect();

    assert_eq!(reasons, expected);
    assert_eq!(RDCleanPathErr::error_code_for_reason("general_error"), None);
}

#[test]
fn reason_error_codes_roundtrip() {
    use der::{Decode as _, Encode as _};

    for &(error_code, _, reason) in REASON_ERROR_CODES {
        let pdu = RDCleanPathPdu {
            error: Some(RDCleanPathErr {
                http_status_code: Some(401),
                ..error_with_code(error_code)
            }),
            ..RDCleanPathPdu::default()
        };
        let encoded = pdu.to_der().unwrap();

        let decoded = RDCleanPathPdu::from_der(&encoded).unwrap();
        assert_eq!(decoded, pdu);

        let RDCleanPathMessage::GeneralErr(error) = decoded.into_message().unwrap() else {
            panic!("{reason} is not decoded as a general error");
        };
        assert_eq!(error.reason(), Some(reason));

        // A peer that predates reason codes sees the same code and HTTP status.
        let deployed = deployed::RDCleanPathPdu::from_der(&encoded).unwrap();
        assert_eq!(deployed.to_der().unwrap(), encoded);
        let deployed_error = deployed.error.unwrap();
        assert_eq!(deployed_error.error_code, error_code);
        assert_eq!(deployed_error.http_status_code, Some(401));
    }
}

#[rstest]
#[case(request())]
#[case(vmconnect_request())]
#[case(response_success())]
#[case(vmconnect_response())]
#[case(response_http_error())]
#[case(response_tls_error())]
#[case(response_reason_error())]
fn deployed_peer_decodes(#[case] message: RDCleanPathPdu) {
    use der::{Decode as _, Encode as _};

    let encoded = message.to_der().unwrap();
    let decoded = deployed::RDCleanPathPdu::from_der(&encoded).unwrap();

    // Re-encoding gives back the same bytes, so the deployed layout kept every field.
    assert_eq!(decoded.to_der().unwrap(), encoded);
}

#[test]
fn deployed_peer_rejects_unknown_error_field() {
    use der::Decode as _;

    // RESPONSE_REASON_ERROR_DER with a hypothetical `[4] UTF8String "expired"` appended to the error.
    // This is why reasons are error codes and not a new field.
    const DER: &[u8] = &[
        0x30, 0x20, 0xA0, 0x4, 0x2, 0x2, 0xD, 0x3E, 0xA1, 0x18, 0x30, 0x16, 0xA0, 0x3, 0x2, 0x1, 0x65, 0xA1, 0x4, 0x2,
        0x2, 0x1, 0x91, 0xA4, 0x9, 0xC, 0x7, 0x65, 0x78, 0x70, 0x69, 0x72, 0x65, 0x64,
    ];

    let error = deployed::RDCleanPathPdu::from_der(DER).unwrap_err();
    assert!(
        matches!(error.kind(), der::ErrorKind::TrailingData { .. }),
        "unexpected error: {error}"
    );
}

#[test]
fn reason_error_display() {
    let descriptions = REASON_ERROR_CODES
        .iter()
        .map(|&(error_code, ..)| error_with_code(error_code).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    expect![["
        invalid token signature (code 100)
        token expired (code 101)
        token not yet valid (code 102)
        token issued for a different gateway (code 103)
        token revoked (code 104)
        token unexpectedly reused (code 105)
        wrong token type (code 106)
        target unreachable (code 107)
        DNS resolution failed (code 108)
        TLS handshake failed (code 109)
        agent unavailable (code 110)"]]
    .assert_eq(&descriptions);
}

/// RDCleanPath structures as deployed peers (ironrdp-rdcleanpath 0.2) define them.
///
/// Keep these frozen: they stand for clients and proxies already in the field.
/// Those peers use der 0.7, which rejects unknown trailing fields the same way.
mod deployed {
    use der::asn1::OctetString;

    #[derive(Debug, der::Sequence)]
    #[asn1(tag_mode = "EXPLICIT")]
    pub(super) struct RDCleanPathErr {
        #[asn1(context_specific = "0")]
        pub(super) error_code: u16,
        #[asn1(context_specific = "1", optional = "true")]
        pub(super) http_status_code: Option<u16>,
        #[asn1(context_specific = "2", optional = "true")]
        pub(super) wsa_last_error: Option<u16>,
        #[asn1(context_specific = "3", optional = "true")]
        pub(super) tls_alert_code: Option<u8>,
    }

    #[derive(Debug, der::Sequence)]
    #[asn1(tag_mode = "EXPLICIT")]
    pub(super) struct RDCleanPathPdu {
        #[asn1(context_specific = "0")]
        pub(super) version: u64,
        #[asn1(context_specific = "1", optional = "true")]
        pub(super) error: Option<RDCleanPathErr>,
        #[asn1(context_specific = "2", optional = "true")]
        pub(super) destination: Option<String>,
        #[asn1(context_specific = "3", optional = "true")]
        pub(super) proxy_auth: Option<String>,
        #[asn1(context_specific = "4", optional = "true")]
        pub(super) server_auth: Option<String>,
        #[asn1(context_specific = "5", optional = "true")]
        pub(super) preconnection_blob: Option<String>,
        #[asn1(context_specific = "6", optional = "true")]
        pub(super) x224_connection_pdu: Option<OctetString>,
        #[asn1(context_specific = "7", optional = "true")]
        pub(super) server_cert_chain: Option<Vec<OctetString>>,
        #[asn1(context_specific = "9", optional = "true")]
        pub(super) server_addr: Option<String>,
    }
}
