use lazy_static::lazy_static;

use super::*;

const DVC_CAPABILITIES_REQUEST_V1_SIZE: usize = 4;
const DVC_CAPABILITIES_REQUEST_V1_BUFFER: [u8; DVC_CAPABILITIES_REQUEST_V1_SIZE] =
    [0x50, 0x00, 0x01, 0x00];

const DVC_CAPABILITIES_REQUEST_V2_SIZE: usize = 12;
const DVC_CAPABILITIES_REQUEST_V2_BUFFER: [u8; DVC_CAPABILITIES_REQUEST_V2_SIZE] = [
    0x50, 0x00, 0x02, 0x00, 0x33, 0x33, 0x11, 0x11, 0x3d, 0x0a, 0xa7, 0x04,
];

const DVC_CAPABILITIES_RESPONSE_SIZE: usize = 4;
const DVC_CAPABILITIES_RESPONSE_BUFFER: [u8; DVC_CAPABILITIES_RESPONSE_SIZE] =
    [0x50, 0x00, 0x01, 0x00];

lazy_static! {
    static ref DVC_CAPABILITIES_REQUEST_V1: CapabilitiesRequestPdu = CapabilitiesRequestPdu::V1;
    static ref DVC_CAPABILITIES_REQUEST_V2: CapabilitiesRequestPdu = CapabilitiesRequestPdu::V2 {
        charges: [0x3333, 0x1111, 0x0a3d, 0x04a7]
    };
    static ref DVC_CAPABILITIES_RESPONSE: CapabilitiesResponsePdu = CapabilitiesResponsePdu {
        version: CapsVersion::V1
    };
}

#[test]
fn from_buffer_parsing_for_dvc_caps_request_pdu_with_invalid_caps_version_fails() {
    let buffer_with_invalid_caps_version = vec![0x00, 0x01, 0x01];
    match CapabilitiesRequestPdu::from_buffer(buffer_with_invalid_caps_version.as_slice()) {
        Err(ChannelError::InvalidDvcCapabilitiesVersion) => (),
        res => panic!(
            "Expected InvalidDvcCapabilitiesVersion error, got: {:?}",
            res
        ),
    };
}

#[test]
fn from_buffer_correct_parses_dvc_capabilities_request_pdu_v1() {
    assert_eq!(
        DVC_CAPABILITIES_REQUEST_V1.clone(),
        CapabilitiesRequestPdu::from_buffer(&DVC_CAPABILITIES_REQUEST_V1_BUFFER[1..]).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_capabilities_request_pdu_v1() {
    let dvc_capabilities_request_pdu_v1 = DVC_CAPABILITIES_REQUEST_V1.clone();

    let mut buffer = Vec::new();
    dvc_capabilities_request_pdu_v1
        .to_buffer(&mut buffer)
        .unwrap();

    assert_eq!(
        DVC_CAPABILITIES_REQUEST_V1_BUFFER.as_ref(),
        buffer.as_slice()
    );
}

#[test]
fn buffer_length_is_correct_for_dvc_capabilities_request_pdu_v1() {
    let dvc_capabilities_request_pdu_v1 = DVC_CAPABILITIES_REQUEST_V1.clone();
    let expected_buf_len = DVC_CAPABILITIES_REQUEST_V1_BUFFER.len();

    let len = dvc_capabilities_request_pdu_v1.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_dvc_capabilities_request_pdu_v2() {
    assert_eq!(
        DVC_CAPABILITIES_REQUEST_V2.clone(),
        CapabilitiesRequestPdu::from_buffer(&DVC_CAPABILITIES_REQUEST_V2_BUFFER[1..]).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_capabilities_request_pdu_v2() {
    let dvc_capabilities_request_pdu_v2 = DVC_CAPABILITIES_REQUEST_V2.clone();

    let mut buffer = Vec::new();
    dvc_capabilities_request_pdu_v2
        .to_buffer(&mut buffer)
        .unwrap();

    assert_eq!(
        DVC_CAPABILITIES_REQUEST_V2_BUFFER.as_ref(),
        buffer.as_slice()
    );
}

#[test]
fn buffer_length_is_correct_for_dvc_capabilities_request_pdu_v2() {
    let dvc_capabilities_request_pdu_v2 = DVC_CAPABILITIES_REQUEST_V2.clone();
    let expected_buf_len = DVC_CAPABILITIES_REQUEST_V2_BUFFER.len();

    let len = dvc_capabilities_request_pdu_v2.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_parsing_for_dvc_caps_response_pdu_with_invalid_caps_version_fails() {
    let buffer_with_invalid_caps_version = vec![0x00, 0x01, 0x01];
    match CapabilitiesResponsePdu::from_buffer(buffer_with_invalid_caps_version.as_slice()) {
        Err(ChannelError::InvalidDvcCapabilitiesVersion) => (),
        res => panic!(
            "Expected InvalidDvcCapabilitiesVersion error, got: {:?}",
            res
        ),
    };
}

#[test]
fn from_buffer_correct_parses_dvc_capabilities_response() {
    assert_eq!(
        DVC_CAPABILITIES_RESPONSE.clone(),
        CapabilitiesResponsePdu::from_buffer(&DVC_CAPABILITIES_RESPONSE_BUFFER[1..]).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_dvc_capabilities_response() {
    let capabilities_response = DVC_CAPABILITIES_RESPONSE.clone();

    let mut buffer = Vec::new();
    capabilities_response.to_buffer(&mut buffer).unwrap();

    assert_eq!(DVC_CAPABILITIES_RESPONSE_BUFFER.as_ref(), buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_dvc_capabilities_response() {
    let capabilities_response = DVC_CAPABILITIES_RESPONSE.clone();
    let expected_buf_len = DVC_CAPABILITIES_RESPONSE_BUFFER.len();

    let len = capabilities_response.buffer_length();

    assert_eq!(expected_buf_len, len);
}
