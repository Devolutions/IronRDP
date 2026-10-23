use ironrdp_core::{decode, encode_vec};
use ironrdp_pdu::mcs::decode_send_data_indication;
use ironrdp_pdu::rdp::client_info::{ClientInfo, PasswordCookie};
use ironrdp_pdu::rdp::headers::{ServerDeactivateAll, ShareControlHeader, ShareControlPdu};
use ironrdp_pdu::rdp::server_redirection::{
    ServerRedirectionFlags, ServerRedirectionPdu, StandardSecurityServerRedirection, decode_server_redirection,
};
use ironrdp_testsuite_core::client_info::CLIENT_INFO_UNICODE;

/// The annotated Enhanced Security Server Redirection PDU of [MS-RDPBCGR] 4.6, TPKT header included.
const ANNOTATED_PDU: &[u8] = include_bytes!("../../test_data/pdu/rdp/enhanced_security_server_redirection.pdu");

/// Offset of the MCS user data (the Share Control Header) in [`ANNOTATED_PDU`]: TPKT (4), X.224 (3), SDin (8).
const USER_DATA_OFFSET: usize = 15;

/// Offset and length of the `Password` field in [`ANNOTATED_PDU`], per the annotation.
const PASSWORD_RANGE: core::ops::Range<usize> = 0xAB..0xAB + 120;

fn annotated_redirection() -> ServerRedirectionPdu {
    let ctx = decode_send_data_indication(ANNOTATED_PDU).expect("Send Data Indication");
    decode_server_redirection(ctx.user_data)
        .expect("valid Server Redirection PDU")
        .expect("a Server Redirection PDU")
}

// oracle: spec MS-RDPBCGR §4.6
#[test]
fn decodes_the_annotated_enhanced_security_server_redirection_pdu() {
    let redirection = annotated_redirection();

    assert_eq!(redirection.session_id, 2);
    assert_eq!(redirection.flags.bits(), 0x0000_0B1D);
    assert_eq!(
        redirection.target_net_address.as_deref(),
        Some("2001:4898:2b:2:9de7:4569:fb39:ef29")
    );
    assert_eq!(redirection.username.as_deref(), Some("administrator"));
    assert_eq!(redirection.domain.as_deref(), Some("TS-STRESS1"));
    assert_eq!(redirection.password.as_deref(), Some(&ANNOTATED_PDU[PASSWORD_RANGE]));
    assert_eq!(
        redirection.target_fqdn.as_deref(),
        Some("jiazou-test2.ts-stress1.nttest.microsoft.com")
    );
    assert_eq!(redirection.target_netbios_name.as_deref(), Some("JIAZOU-TEST2"));
    assert_eq!(
        redirection.target_net_addresses,
        Some(vec![
            "2001:4898:2b:2:9de7:4569:fb39:ef29".to_owned(),
            "157.59.240.144".to_owned()
        ])
    );
    assert_eq!(redirection.load_balance_info, None);
    assert_eq!(redirection.target_certificate, None);
}

// oracle: spec MS-RDPBCGR §4.6
// The padding fields, whose values "MUST be ignored", are expected as zeros.
#[test]
fn encodes_the_annotated_enhanced_security_server_redirection_pdu() {
    let header = ShareControlHeader {
        share_control_pdu: ShareControlPdu::ServerRedirect(annotated_redirection()),
        pdu_source: 1002,
        share_id: 0,
    };

    let mut expected = ANNOTATED_PDU[USER_DATA_OFFSET..].to_vec();
    // pad2Octets after the Share Control Header.
    expected[6..8].fill(0);
    // Pad at the end of the Server Redirection Packet, before pad1Octet.
    let pad_end = expected.len() - 1;
    expected[pad_end - 8..pad_end].fill(0);

    assert_eq!(encode_vec(&header).unwrap(), expected);
}

// oracle: spec MS-RDPBCGR §2.2.13.2.1
// A Basic Security Header carrying SEC_REDIRECTION_PKT (0x0400) with flagsHi zero, then the Server Redirection Packet
// whose Flags field is SEC_REDIRECTION_PKT as well.
#[test]
fn standard_security_server_redirection_is_recognized_on_the_io_channel() {
    let redirection = ServerRedirectionPdu {
        session_id: 3,
        flags: ServerRedirectionFlags::DONT_STORE_USERNAME,
        load_balance_info: Some(b"Cookie: msts=3640205228.15629.0000\r\n".to_vec()),
        username: Some("user".to_owned()),
        target_fqdn: Some("target.example.com".to_owned()),
        ..ServerRedirectionPdu::default()
    };
    let encoded = encode_vec(&StandardSecurityServerRedirection {
        redirection: redirection.clone(),
    })
    .unwrap();

    assert_eq!(encoded[..6], [0x00, 0x04, 0x00, 0x00, 0x00, 0x04]);
    // oracle: spec MS-RDPBCGR §2.2.13.1
    // RedirFlags as sent: LB_LOAD_BALANCE_INFO (0x02) | LB_USERNAME (0x04) | LB_DONTSTOREUSERNAME (0x20) |
    // LB_TARGET_FQDN (0x100).
    let expected = ServerRedirectionPdu {
        flags: ServerRedirectionFlags::from_bits_retain(0x0000_0126),
        ..redirection
    };
    assert_eq!(decode_server_redirection(&encoded).unwrap(), Some(expected));
}

// oracle: spec MS-RDPBCGR §2.2.8.1.1.1.1
// A Server Deactivate All PDU has PDU type 0x6, not PDUTYPE_SERVER_REDIR_PKT (0xA).
#[test]
fn other_share_control_pdus_are_not_server_redirections() {
    let deactivate_all = encode_vec(&ShareControlHeader {
        share_control_pdu: ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll),
        pdu_source: 1002,
        share_id: 0x0001_0000,
    })
    .unwrap();

    assert_eq!(decode_server_redirection(&deactivate_all).unwrap(), None);
}

// oracle: spec MS-RDPBCGR §2.2.13.1.2
// The container layout, Base64-encoded with Python's `base64` module and split by the CR LF line breaks that FreeRDP
// `rdp_redirection_read_base64_wchar` strips.
#[test]
fn target_certificate_is_read_from_base64_with_line_breaks() {
    const BASE64: &str = "AQAAAAAAAAAC\r\nAAAACQkgAAAA\r\nAQAAAAUAAAAw\r\nAwIBBQ==";
    // An unrelated element, then the DER-encoded certificate.
    const CERTIFICATE: [u8; 5] = [0x30, 0x03, 0x02, 0x01, 0x05];

    let text: Vec<u8> = BASE64.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut packet = Vec::new();
    packet.extend_from_slice(&0x0400u16.to_le_bytes()); // Flags
    packet.extend_from_slice(&u16::try_from(16 + text.len()).unwrap().to_le_bytes()); // Length
    packet.extend_from_slice(&9u32.to_le_bytes()); // SessionID
    packet.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // RedirFlags: LB_TARGET_CERTIFICATE
    packet.extend_from_slice(&u32::try_from(text.len()).unwrap().to_le_bytes()); // TargetCertificateLength
    packet.extend_from_slice(&text);

    let redirection: ServerRedirectionPdu = decode(&packet).unwrap();

    let container = redirection.target_certificate.expect("target certificate");
    assert_eq!(container.elements.len(), 2);
    assert_eq!(container.certificate_der(), Some(CERTIFICATE.as_slice()));
}

// oracle: reference awakecoding/FreeRDP@16a7c2ec2fd0:libfreerdp/core/info.c
// `rdp_write_info_packet` sends the cookie as the Password field, cbPassword including the cookie's own trailing null,
// followed by the mandatory null terminator.
#[test]
fn client_info_sends_a_password_cookie_verbatim() {
    // An unpaired UTF-16 surrogate makes the cookie unreadable as text.
    let cookie = vec![0x00, 0xD8, 0x42, 0x00, 0x00, 0x00];
    let client_info = ClientInfo {
        password_cookie: Some(PasswordCookie(cookie.clone())),
        ..CLIENT_INFO_UNICODE.clone()
    };

    let encoded = encode_vec(&client_info).unwrap();
    let decoded: ClientInfo = decode(&encoded).unwrap();

    // CodePage (4), flags (4), cbDomain (2), cbUserName (2), then cbPassword.
    assert_eq!(encoded[12..14], [6, 0]);
    assert_eq!(decoded.password_cookie, Some(PasswordCookie(cookie)));
    assert_eq!(decoded.credentials.password, "");
}

fn decoded_password_cookie(cookie: &[u8]) -> (String, Option<PasswordCookie>) {
    let client_info = ClientInfo {
        password_cookie: Some(PasswordCookie(cookie.to_vec())),
        ..CLIENT_INFO_UNICODE.clone()
    };
    let decoded: ClientInfo = decode(&encode_vec(&client_info).unwrap()).unwrap();
    (decoded.credentials.password, decoded.password_cookie)
}

// oracle: spec MS-RDPBCGR §2.2.13.1
// The Password field is an opaque cookie passed to the target as is, so a cookie whose leading code units read as text
// must not be cut at its first null code unit.
#[test]
fn client_info_keeps_a_password_cookie_with_an_embedded_null() {
    let cookie = [0x41, 0x00, 0x00, 0x00, 0x00, 0xD8, 0x00, 0x00];

    assert_eq!(
        decoded_password_cookie(&cookie),
        (String::new(), Some(PasswordCookie(cookie.to_vec())))
    );
}

// oracle: spec MS-RDPBCGR §4.6
// The annotated cookie is valid UTF-16 text and ends with its own null terminator.
#[test]
fn client_info_keeps_a_valid_utf16_password_cookie() {
    let cookie = &ANNOTATED_PDU[PASSWORD_RANGE];

    assert_eq!(
        decoded_password_cookie(cookie),
        (String::new(), Some(PasswordCookie(cookie.to_vec())))
    );
}

/// A Server Redirection Packet with no optional field and the given `Length`, followed by `trailing` bytes.
fn packet_with_length(length: u16, trailing: usize) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&0x0400u16.to_le_bytes()); // Flags
    packet.extend_from_slice(&length.to_le_bytes()); // Length
    packet.extend_from_slice(&1u32.to_le_bytes()); // SessionID
    packet.extend_from_slice(&0u32.to_le_bytes()); // RedirFlags
    packet.resize(packet.len() + trailing, 0);
    packet
}

// oracle: spec MS-RDPBCGR §2.2.13.1
// Length is the overall length of the packet, which cannot be shorter than its fixed fields.
#[test]
fn server_redirection_length_shorter_than_its_fields_is_rejected() {
    assert!(decode::<ServerRedirectionPdu>(&packet_with_length(8, 0)).is_err());
}

// oracle: spec MS-RDPBCGR §2.2.13.1
// Only the optional 8-byte Pad may be missing from the bytes Length counts.
#[test]
fn server_redirection_length_past_the_padding_is_rejected() {
    assert!(decode::<ServerRedirectionPdu>(&packet_with_length(12 + 9, 0)).is_err());
}

// oracle: reference awakecoding/FreeRDP@16a7c2ec2fd0:libfreerdp/core/redirection.c
// `rdp_recv_server_redirection_pdu` skips the Pad only when present, because "some versions of windows don't included
// this padding before closing the connection".
#[test]
fn server_redirection_without_its_counted_padding_is_accepted() {
    assert!(decode::<ServerRedirectionPdu>(&packet_with_length(12 + 8, 0)).is_ok());
    assert!(decode::<ServerRedirectionPdu>(&packet_with_length(12 + 8, 8)).is_ok());
}
