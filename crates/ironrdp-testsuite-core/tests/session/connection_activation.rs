use std::borrow::Cow;

use ironrdp_connector::connection_activation::{ConnectionActivationSequence, ConnectionActivationState};
use ironrdp_connector::{ClientConnector, ClientConnectorState, Credentials, DesktopSize, Sequence as _, Written};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_pdu::gcc;
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::headers::{ServerDeactivateAll, ShareControlHeader, ShareControlPdu};
use ironrdp_pdu::x224::X224;

use ironrdp_testsuite_core::capsets::SERVER_DEMAND_ACTIVE;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;
const SHARE_ID: u32 = 0x0001_0000;

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

/// Encode a ShareControlPdu as a server-to-client SendDataIndication frame.
fn encode_server_share_control(pdu: ShareControlPdu) -> Vec<u8> {
    let share_control_header = ShareControlHeader {
        share_control_pdu: pdu,
        pdu_source: USER_CHANNEL_ID,
        share_id: SHARE_ID,
    };

    let user_data = encode_vec(&share_control_header).unwrap();

    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: IO_CHANNEL_ID,
        user_data: Cow::Owned(user_data),
    });

    encode_vec(&X224(indication)).unwrap()
}

#[test]
fn deactivate_all_during_capabilities_exchange_stays_in_same_state() {
    let config = test_config();
    let mut seq = ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID);

    let frame = encode_server_share_control(ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll));
    let mut output = WriteBuf::new();

    let written = seq.step(&frame, &mut output).unwrap();

    assert_eq!(written, Written::Nothing);
    assert!(
        matches!(
            seq.connection_activation_state(),
            ConnectionActivationState::CapabilitiesExchange
        ),
        "state should remain CapabilitiesExchange after DeactivateAll"
    );
}

#[test]
fn client_connector_stays_in_capabilities_exchange_on_deactivate_all() {
    let config = test_config();
    let mut connector = ClientConnector::new(config.clone(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::CapabilitiesExchange {
        connection_activation: ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID),
    };

    let frame = encode_server_share_control(ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll));
    let mut output = WriteBuf::new();

    let written = connector.step(&frame, &mut output).unwrap();

    assert_eq!(written, Written::Nothing);
    assert!(
        matches!(connector.state, ClientConnectorState::CapabilitiesExchange { .. }),
        "outer connector state should remain CapabilitiesExchange after DeactivateAll"
    );
}

#[test]
fn demand_active_after_deactivate_all_transitions_to_connection_finalization() {
    let config = test_config();
    let mut seq = ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID);
    let mut output = WriteBuf::new();

    // First: feed DeactivateAll
    let deactivate_frame = encode_server_share_control(ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll));
    let written = seq.step(&deactivate_frame, &mut output).unwrap();
    assert_eq!(written, Written::Nothing);

    // Then: feed ServerDemandActive
    let demand_active_frame =
        encode_server_share_control(ShareControlPdu::ServerDemandActive(SERVER_DEMAND_ACTIVE.clone()));
    let written = seq.step(&demand_active_frame, &mut output).unwrap();

    assert!(written != Written::Nothing, "should have written ClientConfirmActive");
    assert!(
        matches!(
            seq.connection_activation_state(),
            ConnectionActivationState::ConnectionFinalization { .. }
        ),
        "state should transition to ConnectionFinalization after DemandActive"
    );
}
