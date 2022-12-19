use lazy_static::lazy_static;

use super::*;
use crate::rdp;

const ERECT_DOMAIN_PDU_BUFFER_LEN: usize = 5;
const ERECT_DOMAIN_PDU_BUFFER: [u8; ERECT_DOMAIN_PDU_BUFFER_LEN] = [0x04, 0x01, 0x00, 0x01, 0x00];

const ATTACH_USER_REQUEST_PDU_BUFFER_LEN: usize = 1;
const ATTACH_USER_REQUEST_PDU_BUFFER: [u8; ATTACH_USER_REQUEST_PDU_BUFFER_LEN] = [0x28];

const ATTACH_USER_CONFIRM_PDU_BUFFER_LEN: usize = 4;
const ATTACH_USER_CONFIRM_PDU_BUFFER: [u8; ATTACH_USER_CONFIRM_PDU_BUFFER_LEN] = [0x2e, 0x00, 0x00, 0x06];

const CHANNEL_JOIN_REQUEST_PDU_BUFFER_LEN: usize = 5;
const CHANNEL_JOIN_REQUEST_PDU_BUFFER: [u8; CHANNEL_JOIN_REQUEST_PDU_BUFFER_LEN] = [0x38, 0x00, 0x06, 0x03, 0xef];

const CHANNEL_JOIN_CONFIRM_PDU_BUFFER_LEN: usize = 8;
const CHANNEL_JOIN_CONFIRM_PDU_BUFFER: [u8; CHANNEL_JOIN_CONFIRM_PDU_BUFFER_LEN] =
    [0x3e, 0x00, 0x00, 0x06, 0x03, 0xef, 0x03, 0xef];

const DISCONNECT_PROVIDER_ULTIMATUM_PDU_BUFFER_LEN: usize = 2;
const DISCONNECT_PROVIDER_ULTIMATUM_PDU_BUFFER: [u8; DISCONNECT_PROVIDER_ULTIMATUM_PDU_BUFFER_LEN] = [0x21, 0x80];

const SEND_DATA_REQUEST_PDU_BUFFER_PREFIX_LEN: usize = 8;
const SEND_DATA_REQUEST_PDU_BUFFER_PREFIX: [u8; SEND_DATA_REQUEST_PDU_BUFFER_PREFIX_LEN] =
    [0x64, 0x00, 0x06, 0x03, 0xeb, 0x70, 0x81, 0x92];

const SEND_DATA_INDICATION_PDU_BUFFER_PREFIX_LEN: usize = 7;
const SEND_DATA_INDICATION_PDU_BUFFER_PREFIX: [u8; SEND_DATA_INDICATION_PDU_BUFFER_PREFIX_LEN] =
    [0x68, 0x00, 0x01, 0x03, 0xeb, 0x70, 0x14];

const ERECT_DOMAIN_PDU: McsPdu = McsPdu::ErectDomainRequest(ErectDomainPdu {
    sub_height: 0,
    sub_interval: 0,
});
const ATTACH_USER_REQUEST_PDU: McsPdu = McsPdu::AttachUserRequest;
const ATTACH_USER_CONFIRM_PDU: McsPdu = McsPdu::AttachUserConfirm(AttachUserConfirmPdu {
    result: 0,
    initiator_id: 1007,
});
const CHANNEL_JOIN_REQUEST_PDU: McsPdu = McsPdu::ChannelJoinRequest(ChannelJoinRequestPdu {
    initiator_id: 1007,
    channel_id: 1007,
});
const CHANNEL_JOIN_CONFIRM_PDU: McsPdu = McsPdu::ChannelJoinConfirm(ChannelJoinConfirmPdu {
    result: 0,
    initiator_id: 1007,
    requested_channel_id: 1007,
    channel_id: 1007,
});
const DISCONNECT_PROVIDER_ULTIMATUM_PDU: McsPdu =
    McsPdu::DisconnectProviderUltimatum(DisconnectUltimatumReason::UserRequested);

lazy_static! {
    static ref SEND_DATA_REQUEST_PDU_BUFFER: Vec<u8> = {
        let mut result = SEND_DATA_REQUEST_PDU_BUFFER_PREFIX.to_vec();
        result.extend(rdp::test::CLIENT_INFO_PDU_BUFFER.as_slice());

        result
    };
    static ref SEND_DATA_INDICATION_PDU_BUFFER: Vec<u8> = {
        let mut result = SEND_DATA_INDICATION_PDU_BUFFER_PREFIX.to_vec();
        result.extend(Vec::from(rdp::test::SERVER_LICENSE_BUFFER.as_ref()));

        result
    };
    static ref SEND_DATA_REQUEST_PDU: McsPdu = McsPdu::SendDataRequest(SendDataContext {
        initiator_id: 1007,
        channel_id: 1003,
        pdu_length: rdp::test::CLIENT_INFO_PDU_BUFFER.len(),
    });
    static ref SEND_DATA_INDICATION_PDU: McsPdu = McsPdu::SendDataIndication(SendDataContext {
        initiator_id: 1002,
        channel_id: 1003,
        pdu_length: rdp::test::SERVER_LICENSE_BUFFER.len(),
    });
}

#[test]
fn from_buffer_returns_error_with_invalid_domain_mcs_pdu() {
    let buf = vec![0x48, 0x00, 0x00, 0x00, 0x70, 0x00, 0x01, 0x03, 0xEB, 0x70, 0x14];

    match McsPdu::from_buffer(&mut buf.as_slice()) {
        Err(McsError::InvalidDomainMcsPdu) => (),
        _ => panic!("Got invalid result"),
    }
}

#[test]
fn from_buffer_correct_parses_erect_domain_request() {
    let buf = ERECT_DOMAIN_PDU_BUFFER;

    assert_eq!(ERECT_DOMAIN_PDU, McsPdu::from_buffer(buf.as_ref()).unwrap());
}

#[test]
fn to_buffer_correct_serializes_erect_domain_request() {
    let pdu = ERECT_DOMAIN_PDU;
    let expected_buf = ERECT_DOMAIN_PDU_BUFFER;

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_erect_domain_request() {
    let pdu = ERECT_DOMAIN_PDU;
    let expected_buf_len = ERECT_DOMAIN_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_attach_user_request() {
    let buf = ATTACH_USER_REQUEST_PDU_BUFFER;

    assert_eq!(ATTACH_USER_REQUEST_PDU, McsPdu::from_buffer(buf.as_ref()).unwrap());
}

#[test]
fn to_buffer_correct_serializes_attach_user_request() {
    let pdu = ATTACH_USER_REQUEST_PDU;
    let expected_buf = ATTACH_USER_REQUEST_PDU_BUFFER;

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_attach_user_request() {
    let pdu = ATTACH_USER_REQUEST_PDU;
    let expected_buf_len = ATTACH_USER_REQUEST_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_attach_user_confirm() {
    let buf = ATTACH_USER_CONFIRM_PDU_BUFFER;

    assert_eq!(ATTACH_USER_CONFIRM_PDU, McsPdu::from_buffer(buf.as_ref()).unwrap());
}

#[test]
fn to_buffer_correct_serializes_attach_user_confirm() {
    let pdu = ATTACH_USER_CONFIRM_PDU;
    let expected_buf = ATTACH_USER_CONFIRM_PDU_BUFFER;

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_attach_user_confirm() {
    let pdu = ATTACH_USER_CONFIRM_PDU;
    let expected_buf_len = ATTACH_USER_CONFIRM_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_channel_join_request() {
    let buf = CHANNEL_JOIN_REQUEST_PDU_BUFFER;

    assert_eq!(CHANNEL_JOIN_REQUEST_PDU, McsPdu::from_buffer(buf.as_ref()).unwrap());
}

#[test]
fn to_buffer_correct_serializes_channel_join_request() {
    let pdu = CHANNEL_JOIN_REQUEST_PDU;
    let expected_buf = CHANNEL_JOIN_REQUEST_PDU_BUFFER;

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_channel_join_request() {
    let pdu = CHANNEL_JOIN_REQUEST_PDU;
    let expected_buf_len = CHANNEL_JOIN_REQUEST_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_channel_join_confirm() {
    let buf = CHANNEL_JOIN_CONFIRM_PDU_BUFFER;

    assert_eq!(CHANNEL_JOIN_CONFIRM_PDU, McsPdu::from_buffer(buf.as_ref()).unwrap());
}

#[test]
fn to_buffer_correct_serializes_channel_join_confirm() {
    let pdu = CHANNEL_JOIN_CONFIRM_PDU;
    let expected_buf = CHANNEL_JOIN_CONFIRM_PDU_BUFFER;

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_channel_join_confirm() {
    let pdu = CHANNEL_JOIN_CONFIRM_PDU;
    let expected_buf_len = CHANNEL_JOIN_CONFIRM_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_disconnect_ultimatum() {
    let buf = DISCONNECT_PROVIDER_ULTIMATUM_PDU_BUFFER;

    assert_eq!(
        DISCONNECT_PROVIDER_ULTIMATUM_PDU,
        McsPdu::from_buffer(buf.as_ref()).unwrap(),
    );
}

#[test]
fn to_buffer_correct_serializes_disconnect_ultimatum() {
    let pdu = DISCONNECT_PROVIDER_ULTIMATUM_PDU;
    let expected_buf = DISCONNECT_PROVIDER_ULTIMATUM_PDU_BUFFER;

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_disconnect_ultimatum() {
    let pdu = DISCONNECT_PROVIDER_ULTIMATUM_PDU;
    let expected_buf_len = DISCONNECT_PROVIDER_ULTIMATUM_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_send_data_request() {
    let buf = SEND_DATA_REQUEST_PDU_BUFFER.to_vec();

    assert_eq!(
        SEND_DATA_REQUEST_PDU.clone(),
        McsPdu::from_buffer(buf.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correct_serializes_send_data_request() {
    let pdu = SEND_DATA_REQUEST_PDU.clone();
    let expected_buf = SEND_DATA_REQUEST_PDU_BUFFER_PREFIX.to_vec();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn buffer_length_is_correct_for_send_data_request() {
    let pdu = SEND_DATA_REQUEST_PDU.clone();
    let expected_buf_len = SEND_DATA_REQUEST_PDU_BUFFER_PREFIX.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn from_buffer_correct_parses_send_data_indication() {
    let buf = SEND_DATA_INDICATION_PDU_BUFFER.to_vec();

    assert_eq!(
        SEND_DATA_INDICATION_PDU.clone(),
        McsPdu::from_buffer(buf.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correct_serializes_send_data_indication() {
    let pdu = SEND_DATA_INDICATION_PDU.clone();
    let expected_buf = SEND_DATA_INDICATION_PDU_BUFFER_PREFIX.to_vec();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn buffer_length_is_correct_for_send_data_indication() {
    let pdu = SEND_DATA_INDICATION_PDU.clone();
    let expected_buf_len = SEND_DATA_INDICATION_PDU_BUFFER_PREFIX.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}
