use std::borrow::Cow;

use ironrdp_connector::connection_activation::{ConnectionActivationSequence, ConnectionActivationState};
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, Credentials, DesktopSize, MultitransportResult, Sequence as _, Written,
};
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_pdu::gcc;
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::headers::{
    BasicSecurityHeader, BasicSecurityHeaderFlags, ServerDeactivateAll, ShareControlHeader, ShareControlPdu,
};
use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, RequestedProtocol};
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
            ConnectionActivationState::CapabilitiesExchange { .. }
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

fn multitransport_request(request_id: u32, requested_protocol: RequestedProtocol) -> MultitransportRequestPdu {
    MultitransportRequestPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
        },
        request_id,
        requested_protocol,
        security_cookie: [0u8; 16],
    }
}

/// A connector parked in `MultitransportPending` with `requests` outstanding and
/// a real Demand Active buffered, ready for `complete`/`skip` to replay.
fn multitransport_pending_connector(requests: Vec<MultitransportRequestPdu>) -> ClientConnector {
    let buffered_demand_active =
        encode_server_share_control(ShareControlPdu::ServerDemandActive(SERVER_DEMAND_ACTIVE.clone()));
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::MultitransportPending {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
        requests,
        buffered_demand_active,
    };
    connector
}

#[test]
fn should_perform_multitransport_reflects_pending_state() {
    let connector = multitransport_pending_connector(vec![multitransport_request(1, RequestedProtocol::UdpFecR)]);
    assert!(connector.should_perform_multitransport());
    assert_eq!(connector.multitransport_requests().len(), 1);
}

#[test]
fn complete_multitransport_replays_demand_active_and_advances() {
    let mut connector = multitransport_pending_connector(vec![
        multitransport_request(1, RequestedProtocol::UdpFecR),
        multitransport_request(2, RequestedProtocol::UdpFecL),
    ]);
    let mut output = WriteBuf::new();

    let written = connector
        .complete_multitransport(
            &[MultitransportResult::Success, MultitransportResult::Success],
            &mut output,
        )
        .unwrap();

    assert!(
        written != Written::Nothing,
        "responses plus the replayed ClientConfirmActive"
    );
    assert!(
        matches!(connector.state, ClientConnectorState::ConnectionFinalization { .. }),
        "connector must advance past the buffered Demand Active on completion"
    );
    assert!(!connector.should_perform_multitransport());
}

#[test]
fn complete_multitransport_carries_failure_results() {
    let mut connector = multitransport_pending_connector(vec![multitransport_request(7, RequestedProtocol::UdpFecR)]);
    let mut output = WriteBuf::new();

    let written = connector
        .complete_multitransport(&[MultitransportResult::Failure(0x8000_0001)], &mut output)
        .unwrap();

    assert!(written != Written::Nothing);
    assert!(matches!(
        connector.state,
        ClientConnectorState::ConnectionFinalization { .. }
    ));
}

#[test]
fn skip_multitransport_replays_demand_active_and_advances() {
    let mut connector = multitransport_pending_connector(vec![multitransport_request(1, RequestedProtocol::UdpFecR)]);
    let mut output = WriteBuf::new();

    let written = connector.skip_multitransport(&mut output).unwrap();

    assert!(written != Written::Nothing, "the replayed ClientConfirmActive");
    assert!(matches!(
        connector.state,
        ClientConnectorState::ConnectionFinalization { .. }
    ));
    assert!(!connector.should_perform_multitransport());
}

#[test]
fn complete_multitransport_rejects_result_count_mismatch() {
    let mut connector = multitransport_pending_connector(vec![multitransport_request(1, RequestedProtocol::UdpFecR)]);
    let mut output = WriteBuf::new();

    // One outstanding request, zero results provided.
    assert!(connector.complete_multitransport(&[], &mut output).is_err());
}

#[test]
fn complete_multitransport_outside_pending_state_errors() {
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::CapabilitiesExchange {
        connection_activation: ConnectionActivationSequence::new(test_config(), IO_CHANNEL_ID, USER_CHANNEL_ID),
    };
    let mut output = WriteBuf::new();

    assert!(connector.complete_multitransport(&[], &mut output).is_err());
}

#[test]
fn skip_multitransport_outside_pending_state_errors() {
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::CapabilitiesExchange {
        connection_activation: ConnectionActivationSequence::new(test_config(), IO_CHANNEL_ID, USER_CHANNEL_ID),
    };
    let mut output = WriteBuf::new();

    assert!(connector.skip_multitransport(&mut output).is_err());
}

#[test]
fn demand_active_user_data_does_not_decode_as_multitransport_request() {
    // The connector distinguishes a multitransport request from a Demand Active
    // by try-decoding the SendDataIndication user_data as MultitransportRequestPdu.
    // A Demand Active must fail cleanly (the decoder validates SEC_TRANSPORT_REQ),
    // otherwise the bootstrapping state would swallow the Demand Active.
    let share_control_header = ShareControlHeader {
        share_control_pdu: ShareControlPdu::ServerDemandActive(SERVER_DEMAND_ACTIVE.clone()),
        pdu_source: USER_CHANNEL_ID,
        share_id: SHARE_ID,
    };
    let user_data = encode_vec(&share_control_header).unwrap();

    assert!(
        decode::<MultitransportRequestPdu>(&user_data).is_err(),
        "a Demand Active must not be mistaken for a multitransport request"
    );
}
