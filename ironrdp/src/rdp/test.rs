use lazy_static::lazy_static;

use super::*;
use crate::rdp::{
    client_info::test::{CLIENT_INFO_BUFFER_UNICODE, CLIENT_INFO_UNICODE},
    client_license::test::{LICENSE_PACKET, LICENSE_PACKET_BUFFER},
};

const CLIENT_INFO_PDU_BUFFER_PREFIX: [u8; 4] = [0x40, 0x00, 0x00, 0x00];
const CLIENT_LICENSE_PDU_BUFFER_PREFIX: [u8; 4] = [0x80, 0x00, 0x00, 0x00];

lazy_static! {
    pub static ref CLIENT_INFO_PDU: ClientInfoPdu = ClientInfoPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::INFO_PKT,
        },
        client_info: CLIENT_INFO_UNICODE.clone(),
    };
    pub static ref CLIENT_LICENSE_PDU: ClientLicensePdu = ClientLicensePdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::LICENSE_PKT,
        },
        client_license: LICENSE_PACKET.clone(),
    };
    pub static ref CLIENT_INFO_PDU_BUFFER: Vec<u8> = {
        let mut buffer = CLIENT_INFO_PDU_BUFFER_PREFIX.to_vec();
        buffer.extend(CLIENT_INFO_BUFFER_UNICODE.as_ref());

        buffer
    };
    pub static ref CLIENT_LICENSE_PDU_BUFFER: Vec<u8> = {
        let mut buffer = CLIENT_LICENSE_PDU_BUFFER_PREFIX.to_vec();
        buffer.extend(LICENSE_PACKET_BUFFER.as_ref());

        buffer
    };
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_client_info() {
    let buf = CLIENT_INFO_PDU_BUFFER.clone();

    assert_eq!(
        CLIENT_INFO_PDU.clone(),
        ClientInfoPdu::from_buffer(buf.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correct_parses_rdp_pdu_client_license() {
    let buf = CLIENT_LICENSE_PDU_BUFFER.clone();

    assert_eq!(
        CLIENT_LICENSE_PDU.clone(),
        ClientLicensePdu::from_buffer(buf.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_client_info() {
    let pdu = CLIENT_INFO_PDU.clone();
    let expected_buf = CLIENT_INFO_PDU_BUFFER.clone();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn to_buffer_correct_serializes_rdp_pdu_client_license() {
    let pdu = CLIENT_LICENSE_PDU.clone();
    let expected_buf = CLIENT_LICENSE_PDU_BUFFER.clone();

    let mut buf = Vec::new();
    pdu.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buf, buf);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_info() {
    let pdu = CLIENT_INFO_PDU.clone();
    let expected_buf_len = CLIENT_INFO_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}

#[test]
fn buffer_length_is_correct_for_rdp_pdu_client_license() {
    let pdu = CLIENT_LICENSE_PDU.clone();
    let expected_buf_len = CLIENT_LICENSE_PDU_BUFFER.len();

    let len = pdu.buffer_length();

    assert_eq!(expected_buf_len, len);
}
