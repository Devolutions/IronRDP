//! Connect-time auto-detection demux in the client connector.
//!
//! The continuous (session) auto-detect path is covered in
//! `tests/session/autodetect.rs`. These tests cover the connector's
//! `ConnectTimeAutoDetection` state, which demultiplexes the first PDU received
//! once a message channel has been negotiated: an Auto-Detect Request on the
//! message channel is answered, any other message-channel PDU is ignored, and a
//! PDU on the I/O channel is the first licensing PDU.

use std::borrow::Cow;

use ironrdp_connector::{ClientConnector, ClientConnectorState, Credentials, DesktopSize, Sequence as _, Written};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_pdu::gcc;
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::autodetect::{AutoDetectReqPdu, AutoDetectRequest};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use ironrdp_pdu::rdp::server_license::{
    LicenseErrorCode, LicenseHeader, LicensePdu, LicensingErrorMessage, LicensingStateTransition, PreambleFlags,
    PreambleType, PreambleVersion,
};
use ironrdp_pdu::x224::X224;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;
const MESSAGE_CHANNEL_ID: u16 = 1004;

fn test_config() -> ironrdp_connector::Config {
    ironrdp_connector::Config {
        desktop_size: DesktopSize {
            width: 1024,
            height: 768,
        },
        desktop_scale_factor: 0,
        enable_tls: true,
        enable_credssp: false,
        credentials: Credentials::UsernamePassword {
            username: "test".into(),
            password: "test".into(),
        },
        domain: None,
        client_build: 0,
        client_name: "test".into(),
        keyboard_type: gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        license_cache: None,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
        multitransport_flags: None,
        performance_flags: Default::default(),
        timezone_info: Default::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

/// A client connector parked in `ConnectTimeAutoDetection` with a negotiated
/// message channel, ready to receive the first PDU of that phase.
fn connect_time_autodetect_connector() -> ClientConnector {
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:12345".parse().unwrap());
    connector.state = ClientConnectorState::ConnectTimeAutoDetection {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
    };
    connector.message_channel_id = Some(MESSAGE_CHANNEL_ID);
    connector
}

/// Frame a server-to-client SendDataIndication on the given MCS channel.
fn server_send_data_indication(channel_id: u16, user_data: Vec<u8>) -> Vec<u8> {
    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id,
        user_data: Cow::Owned(user_data),
    });

    encode_vec(&X224(indication)).unwrap()
}

#[test]
fn connect_time_autodetect_request_is_answered_and_phase_continues() {
    let mut connector = connect_time_autodetect_connector();

    let user_data = encode_vec(&AutoDetectReqPdu::new(AutoDetectRequest::rtt_connect_time(0x1234))).unwrap();
    let frame = server_send_data_indication(MESSAGE_CHANNEL_ID, user_data);

    let mut output = WriteBuf::new();
    let written = connector.step(&frame, &mut output).unwrap();

    assert!(written.size().is_some(), "an RTT request must produce a response frame");
    assert!(
        matches!(connector.state, ClientConnectorState::ConnectTimeAutoDetection { .. }),
        "the connector keeps listening after answering an auto-detect request"
    );
}

#[test]
fn unrelated_message_channel_pdu_is_ignored_and_phase_continues() {
    let mut connector = connect_time_autodetect_connector();

    // A message-channel PDU that is not an auto-detect request: a bare security
    // header without the SEC_AUTODETECT_REQ flag. It must be ignored, not handed
    // to the licensing sequence (which would try to decode it as a license PDU).
    let user_data = encode_vec(&BasicSecurityHeader {
        flags: BasicSecurityHeaderFlags::HEARTBEAT,
    })
    .unwrap();
    let frame = server_send_data_indication(MESSAGE_CHANNEL_ID, user_data);

    let mut output = WriteBuf::new();
    let written = connector.step(&frame, &mut output).unwrap();

    assert_eq!(
        written,
        Written::Nothing,
        "an unrelated message-channel PDU produces no response"
    );
    assert!(
        matches!(connector.state, ClientConnectorState::ConnectTimeAutoDetection { .. }),
        "the connector keeps listening on the message channel"
    );
}

#[test]
fn first_licensing_pdu_leaves_autodetect_for_the_licensing_path() {
    let mut connector = connect_time_autodetect_connector();

    // The first PDU that is not on the message channel is the licensing PDU on
    // the I/O channel. A STATUS_VALID_CLIENT license error completes licensing in
    // a single step ([MS-RDPELE] 3.1.5.3.1), so the connector advances out of
    // auto-detection into multitransport bootstrapping.
    let license = LicensePdu::LicensingErrorMessage(LicensingErrorMessage {
        license_header: LicenseHeader {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::LICENSE_PKT,
            },
            preamble_message_type: PreambleType::ErrorAlert,
            preamble_flags: PreambleFlags::empty(),
            preamble_version: PreambleVersion::V3,
            preamble_message_size: 0x10,
        },
        error_code: LicenseErrorCode::StatusValidClient,
        state_transition: LicensingStateTransition::NoTransition,
        error_info: Vec::new(),
    });
    let user_data = encode_vec(&license).unwrap();
    let frame = server_send_data_indication(IO_CHANNEL_ID, user_data);

    let mut output = WriteBuf::new();
    connector.step(&frame, &mut output).unwrap();

    assert!(
        matches!(
            connector.state,
            ClientConnectorState::MultitransportBootstrapping { .. }
        ),
        "a completed licensing exchange advances the connector out of auto-detection"
    );
}
