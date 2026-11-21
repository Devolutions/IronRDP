use ironrdp_core::{decode, encode_vec};
use ironrdp_pdu::rdp::rdstls::{
    RDSTLS_VERSION_1, RdstlsAuthenticationResponse, RdstlsCapabilities, RdstlsPasswordAuthenticationRequest,
    RdstlsResultCode,
};
use proptest::prelude::*;

/// RDSTLS Capabilities PDU advertising version 1 only.
const CAPABILITIES: &[u8] = &[
    0x01, 0x00, // Version = RDSTLS_VERSION_1
    0x01, 0x00, // PduType = RDSTLS_TYPE_CAPABILITIES
    0x01, 0x00, // DataType = RDSTLS_DATA_CAPABILITIES
    0x01, 0x00, // SupportedVersions = RDSTLS_VERSION_1
];

fn authentication_response(result_code: u32) -> Vec<u8> {
    let mut pdu = vec![
        0x01, 0x00, // Version = RDSTLS_VERSION_1
        0x04, 0x00, // PduType = RDSTLS_TYPE_AUTHRSP
        0x01, 0x00, // DataType = RDSTLS_DATA_RESULT_CODE
    ];
    pdu.extend_from_slice(&result_code.to_le_bytes()); // ResultCode
    pdu
}

// oracle: spec MS-RDPBCGR §2.2.17.1
#[test]
fn capabilities_are_decoded() {
    let capabilities: RdstlsCapabilities = decode(CAPABILITIES).expect("valid RDSTLS Capabilities PDU");

    assert_eq!(capabilities.supported_versions, RDSTLS_VERSION_1);
}

// oracle: spec MS-RDPBCGR §2.2.17.4
#[rstest::rstest]
#[case::success(0x0000_0000, RdstlsResultCode::SUCCESS)]
#[case::access_denied(0x0000_0005, RdstlsResultCode::ACCESS_DENIED)]
#[case::logon_failure(0x0000_052E, RdstlsResultCode::LOGON_FAILURE)]
#[case::invalid_logon_hours(0x0000_0530, RdstlsResultCode::INVALID_LOGON_HOURS)]
#[case::password_expired(0x0000_0532, RdstlsResultCode::PASSWORD_EXPIRED)]
#[case::account_disabled(0x0000_0533, RdstlsResultCode::ACCOUNT_DISABLED)]
#[case::password_must_change(0x0000_0773, RdstlsResultCode::PASSWORD_MUST_CHANGE)]
#[case::account_locked_out(0x0000_0775, RdstlsResultCode::ACCOUNT_LOCKED_OUT)]
fn authentication_response_result_code_is_decoded(#[case] wire: u32, #[case] expected: RdstlsResultCode) {
    let response: RdstlsAuthenticationResponse =
        decode(&authentication_response(wire)).expect("valid RDSTLS Authentication Response PDU");

    assert_eq!(response.result_code, expected);
}

// oracle: spec MS-RDPBCGR §2.2.17.4
// PduType "MUST be set to RDSTLS_TYPE_AUTHRSP (0x0004)"; here it is RDSTLS_TYPE_CAPABILITIES in a PDU of the right size.
#[test]
fn authentication_response_with_another_pdu_type_is_rejected() {
    /// Offset of `PduType`: it follows the 2-byte `Version`.
    const PDU_TYPE_OFFSET: usize = 2;

    let mut response = authentication_response(0);
    response[PDU_TYPE_OFFSET..PDU_TYPE_OFFSET + 2].copy_from_slice(&[0x01, 0x00]);

    assert!(decode::<RdstlsAuthenticationResponse>(&response).is_err());
}

// oracle: spec MS-RDPBCGR §2.2.17.2
// UserName and Domain are Unicode with a null terminator that their length counts; the GUID and the password are
// copied verbatim.
#[test]
fn password_authentication_request_is_encoded() {
    let request = RdstlsPasswordAuthenticationRequest {
        redirection_guid: b"G\0U\0".to_vec(),
        username: "us".to_owned(),
        domain: "d".to_owned(),
        password: vec![0xDE, 0xAD, 0xBE],
    };

    let expected: &[u8] = &[
        0x01, 0x00, // Version = RDSTLS_VERSION_1
        0x02, 0x00, // PduType = RDSTLS_TYPE_AUTHREQ
        0x01, 0x00, // DataType = RDSTLS_DATA_PASSWORD_CREDS
        0x04, 0x00, // RedirectionGuidLength
        b'G', 0x00, b'U', 0x00, // RedirectionGuid
        0x06, 0x00, // UserNameLength
        b'u', 0x00, b's', 0x00, 0x00, 0x00, // UserName
        0x04, 0x00, // DomainLength
        b'd', 0x00, 0x00, 0x00, // Domain
        0x03, 0x00, // PasswordLength
        0xDE, 0xAD, 0xBE, // Password
    ];
    assert_eq!(encode_vec(&request).expect("encodable request"), expected);
}

// oracle: reference FreeRDP/FreeRDP@12d8399cf44c:libfreerdp/core/rdstls.c
// `rdstls_write_string` sends an absent user name or domain as a lone null terminator.
#[test]
fn empty_domain_is_sent_as_a_null_terminator() {
    let request = RdstlsPasswordAuthenticationRequest {
        redirection_guid: Vec::new(),
        username: "u".to_owned(),
        domain: String::new(),
        password: vec![0x01],
    };

    let encoded = encode_vec(&request).expect("encodable request");

    let domain_offset = 2 /* Version */ + 2 /* PduType */ + 2 /* DataType */ + 2 /* RedirectionGuidLength */
        + 2 /* UserNameLength */ + 4 /* UserName */;
    assert_eq!(&encoded[domain_offset..domain_offset + 4], &[0x02, 0x00, 0x00, 0x00]);
}

fn any_password_authentication_request() -> impl Strategy<Value = RdstlsPasswordAuthenticationRequest> {
    (
        proptest::collection::vec(any::<u8>(), 0..64),
        "[^\0]{0,64}",
        "[^\0]{0,16}",
        proptest::collection::vec(any::<u8>(), 1..512),
    )
        .prop_map(
            |(redirection_guid, username, domain, password)| RdstlsPasswordAuthenticationRequest {
                redirection_guid,
                username,
                domain,
                password,
            },
        )
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn password_authentication_request_round_trips(request in any_password_authentication_request()) {
        let encoded = encode_vec(&request).unwrap();

        prop_assert_eq!(decode::<RdstlsPasswordAuthenticationRequest>(&encoded).unwrap(), request);
    }

    // oracle: property round-trip
    #[test]
    fn authentication_response_round_trips(result_code in any::<u32>()) {
        let response = RdstlsAuthenticationResponse { result_code: RdstlsResultCode(result_code) };

        prop_assert_eq!(decode::<RdstlsAuthenticationResponse>(&encode_vec(&response).unwrap()).unwrap(), response);
    }
}
