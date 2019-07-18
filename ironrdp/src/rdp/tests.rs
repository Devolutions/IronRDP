use super::*;

use std::io;

use crate::tpdu::X224TPDUType;

#[test]
fn fastpath_header_with_long_len_is_parsed_correctly() {
    let buf = vec![0x9C, 0x81, 0xE7];

    let (fastpath, length) = parse_fastpath_header(&mut buf.as_slice()).unwrap();

    assert_eq!(fastpath.encryption_flags, 0x02);
    assert_eq!(fastpath.number_events, 7);
    assert_eq!(fastpath.length, 484);
    assert_eq!(length, 487);
}

#[test]
fn fastpath_header_with_short_len_is_parsed_correctly() {
    let buf = vec![0x8B, 0x08];

    let (fastpath, length) = parse_fastpath_header(&mut buf.as_slice()).unwrap();

    assert_eq!(fastpath.encryption_flags, 0x02);
    assert_eq!(fastpath.number_events, 2);
    assert_eq!(fastpath.length, 6);
    assert_eq!(length, 8);
}

#[test]
fn erect_domain_request_is_parsed_correctly() {
    let buf = vec![0x04, 0x01, 0x00, 0x01, 0x00];

    match parse_rdp_header(&mut buf.as_slice(), X224TPDUType::Data).unwrap() {
        RdpHeaderMessage::ErectDomainRequest => (),
        _ => panic!("Invalid RDP header message type"),
    }
}

#[test]
fn attach_user_request_is_parsed_correctly() {
    let buf = vec![0x28];

    match parse_rdp_header(&mut buf.as_slice(), X224TPDUType::Data).unwrap() {
        RdpHeaderMessage::AttachUserRequest => (),
        _ => panic!("Invalid RDP header message type"),
    }
}

#[test]
fn attach_user_confirm_is_parsed_correctly() {
    let buf = vec![0x2e, 0x00, 0x00, 0x07];

    match parse_rdp_header(&mut buf.as_slice(), X224TPDUType::Data).unwrap() {
        RdpHeaderMessage::AttachUserId(user_id) => {
            assert_eq!(user_id, 1008);
        }
        _ => panic!("Invalid RDP header message type"),
    }
}

#[test]
fn channel_join_request_is_parsed_correctly() {
    let buf = vec![0x38, 0x00, 0x07, 0x03, 0xeb];

    match parse_rdp_header(&mut buf.as_slice(), X224TPDUType::Data).unwrap() {
        RdpHeaderMessage::ChannelIdJoinRequest(channel_id) => {
            assert_eq!(channel_id, 1003);
        }
        _ => panic!("Invalid RDP header message type"),
    }
}

#[test]
fn channel_join_confirm_is_parsed_correctly() {
    let buf = vec![0x3e, 0x00, 0x00, 0x07, 0x03, 0xf0, 0x03, 0xf0];

    match parse_rdp_header(&mut buf.as_slice(), X224TPDUType::Data).unwrap() {
        RdpHeaderMessage::ChannelIdJoinConfirm(channel_id) => {
            assert_eq!(channel_id, 1008);
        }
        _ => panic!("Invalid RDP header message type"),
    }
}

#[test]
fn rdp_header_is_parsed_correctly() {
    let buf = vec![0x68, 0x00, 0x01, 0x03, 0xEB, 0x70, 0x14];

    match parse_rdp_header(&mut buf.as_slice(), X224TPDUType::Data).unwrap() {
        RdpHeaderMessage::SendData(SendDataContext { length, channel_id }) => {
            assert_eq!(length, 20);
            assert_eq!(channel_id, 1003);
        }
        _ => panic!("Invalid RDP header message type"),
    }
}

#[test]
fn disconnect_ultimatum_is_parsed_correctly() {
    let buf = vec![0x21, 0x80];

    let result = parse_rdp_header(&mut buf.as_slice(), X224TPDUType::Data).unwrap();

    assert_eq!(
        result,
        RdpHeaderMessage::DisconnectProviderUltimatum(DisconnectUltimatumReason::UserRequested)
    );
}

#[test]
fn parse_rdp_header_returns_error_with_invalid_x224_code() {
    let buf = vec![0x21, 0x80];

    let result = parse_rdp_header(&mut buf.as_slice(), X224TPDUType::ConnectionRequest);

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[test]
fn parse_rdp_header_returns_error_with_invalid_mcspdu() {
    let buf = vec![0x70, 0x00, 0x01, 0x03, 0xEB, 0x70, 0x14];

    let result = parse_rdp_header(&mut buf.as_slice(), X224TPDUType::Data);

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
}
