use std::borrow::Cow;

use ironrdp_connector::connection_activation::{ConnectionActivationSequence, ConnectionActivationState};
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, Credentials, DesktopSize, MultitransportResult, Sequence as _, Written,
};
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_pdu::gcc;
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::capability_sets::{CapabilitySet, MajorPlatformType};
use ironrdp_pdu::rdp::client_info::CompressionType;
use ironrdp_pdu::rdp::headers::{
    BasicSecurityHeader, BasicSecurityHeaderFlags, CompressionFlags, ServerDeactivateAll, ShareControlHeader,
    ShareControlPdu, ShareDataHeader, ShareDataPdu, StreamPriority,
};
use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, MultitransportResponsePdu, RequestedProtocol};
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use ironrdp_pdu::x224::X224;
use ironrdp_session::x224::{Processor, ProcessorOutput};
use ironrdp_svc::StaticChannelSet;

use ironrdp_testsuite_core::capsets::SERVER_DEMAND_ACTIVE;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;
const MESSAGE_CHANNEL_ID: u16 = 1004;
const SHARE_ID: u32 = 0x0001_0000;

fn test_config() -> ironrdp_connector::Config {
    ironrdp_connector::Config {
        desktop_size: DesktopSize {
            width: 1024,
            height: 768,
        },
        monitor_layout: None,
        desktop_scale_factor: 0,
        enable_tls: true,
        enable_credssp: false,
        enable_standard_rdp_security: false,
        credentials: Credentials::UsernamePassword {
            username: "test".into(),
            password: "test".into(),
        },
        domain: None,
        client_build: 0,
        client_name: "test".into(),
        keyboard_type: gcc::KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        connection_type: gcc::ConnectionType::Lan,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        enable_audio_capture: false,
        license_cache: None,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
        performance_flags: Default::default(),
        timezone_info: Default::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel::SUPPORTED,
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

fn demand_active_static_channel_chunk_size(chunk_size: Option<u32>) -> usize {
    let mut demand_active = SERVER_DEMAND_ACTIVE.clone();
    let virtual_channel = demand_active
        .pdu
        .capability_sets
        .iter_mut()
        .find_map(|capability_set| match capability_set {
            CapabilitySet::VirtualChannel(virtual_channel) => Some(virtual_channel),
            _ => None,
        })
        .expect("server demand active should include a virtual channel capability");
    virtual_channel.chunk_size = chunk_size;

    let mut sequence = ConnectionActivationSequence::new(test_config(), IO_CHANNEL_ID, USER_CHANNEL_ID);
    let mut output = WriteBuf::new();
    let frame = encode_server_share_control(ShareControlPdu::ServerDemandActive(demand_active));
    sequence
        .step(&frame, None, &mut output)
        .expect("demand active should be accepted");

    match sequence.connection_activation_state() {
        ConnectionActivationState::ConnectionFinalization {
            static_channel_chunk_size,
            ..
        } => static_channel_chunk_size,
        state => panic!("expected ConnectionFinalization, got {state:?}"),
    }
}

#[test]
fn demand_active_uses_valid_server_static_channel_chunk_size() {
    for chunk_size in [
        u32::try_from(ironrdp_svc::CHANNEL_CHUNK_LENGTH).expect("chunk size should fit in u32"),
        u32::try_from(ironrdp_svc::MAX_CHANNEL_CHUNK_LENGTH).expect("chunk size should fit in u32"),
    ] {
        assert_eq!(
            demand_active_static_channel_chunk_size(Some(chunk_size)),
            usize::try_from(chunk_size).expect("chunk size should fit in usize"),
        );
    }
}

#[test]
fn demand_active_falls_back_to_default_static_channel_chunk_size() {
    for chunk_size in [
        None,
        Some(u32::try_from(ironrdp_svc::CHANNEL_CHUNK_LENGTH).expect("chunk size should fit in u32") - 1),
        Some(u32::try_from(ironrdp_svc::MAX_CHANNEL_CHUNK_LENGTH).expect("chunk size should fit in u32") + 1),
    ] {
        assert_eq!(
            demand_active_static_channel_chunk_size(chunk_size),
            ironrdp_svc::CHANNEL_CHUNK_LENGTH,
        );
    }
}

#[test]
fn deactivate_all_during_capabilities_exchange_stays_in_same_state() {
    let config = test_config();
    let mut seq = ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID);

    let frame = encode_server_share_control(ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll));
    let mut output = WriteBuf::new();

    let written = seq.step(&frame, None, &mut output).unwrap();

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

    let written = connector.step(&frame, None, &mut output).unwrap();

    assert_eq!(written, Written::Nothing);
    assert!(
        matches!(connector.state, ClientConnectorState::CapabilitiesExchange { .. }),
        "outer connector state should remain CapabilitiesExchange after DeactivateAll"
    );
}

#[test]
fn set_error_info_during_capabilities_exchange_surfaces_the_disconnect_reason() {
    // Instead of reactivating after a Deactivate-All, a server may end the session
    // (MS-RDPBCGR §1.3.1.3) by sending a Set Error Info PDU carrying the disconnect
    // reason. The sequence must surface that reason rather than a generic
    // "unexpected Share Control PDU" error.
    let config = test_config();
    let mut seq = ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID);

    let error_info = ShareControlPdu::Data(ShareDataHeader {
        share_data_pdu: ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
            ProtocolIndependentCode::RpcInitiatedDisconnect,
        ))),
        stream_priority: StreamPriority::Medium,
        compression_flags: CompressionFlags::empty(),
        compression_type: CompressionType::K8,
    });
    let frame = encode_server_share_control(error_info);
    let mut output = WriteBuf::new();

    let err = seq
        .step(&frame, None, &mut output)
        .expect_err("a Set Error Info PDU during capabilities exchange must end the sequence with an error");

    let message = err.to_string();
    assert!(
        message.contains("error info"),
        "the error should surface the server's disconnect reason, got: {message}"
    );
    assert!(
        !message.contains("unexpected Share Control PDU"),
        "the disconnect reason must not be masked by the generic unexpected-PDU error, got: {message}"
    );
}

#[test]
fn none_error_info_during_capabilities_exchange_is_skipped() {
    // An ERRINFO_NONE Set Error Info PDU is informational, not a disconnect. It must
    // be skipped (staying in Capabilities Exchange to await Demand Active), not treated
    // as a session-ending error.
    let config = test_config();
    let mut seq = ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID);

    let none_error_info = ShareControlPdu::Data(ShareDataHeader {
        share_data_pdu: ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
            ProtocolIndependentCode::None,
        ))),
        stream_priority: StreamPriority::Medium,
        compression_flags: CompressionFlags::empty(),
        compression_type: CompressionType::K8,
    });
    let frame = encode_server_share_control(none_error_info);
    let mut output = WriteBuf::new();

    let written = seq.step(&frame, None, &mut output).unwrap();

    assert_eq!(written, Written::Nothing);
    assert!(
        matches!(
            seq.connection_activation_state(),
            ConnectionActivationState::CapabilitiesExchange
        ),
        "state should remain CapabilitiesExchange after a benign ERRINFO_NONE PDU"
    );
}

#[test]
fn demand_active_after_deactivate_all_transitions_to_connection_finalization() {
    let config = test_config();
    let mut seq = ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID);
    let mut output = WriteBuf::new();

    // First: feed DeactivateAll
    let deactivate_frame = encode_server_share_control(ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll));
    let written = seq.step(&deactivate_frame, None, &mut output).unwrap();
    assert_eq!(written, Written::Nothing);

    // Then: feed ServerDemandActive
    let demand_active_frame =
        encode_server_share_control(ShareControlPdu::ServerDemandActive(SERVER_DEMAND_ACTIVE.clone()));
    let written = seq.step(&demand_active_frame, None, &mut output).unwrap();

    assert!(written != Written::Nothing, "should have written ClientConfirmActive");
    assert!(
        matches!(
            seq.connection_activation_state(),
            ConnectionActivationState::ConnectionFinalization { .. }
        ),
        "state should transition to ConnectionFinalization after DemandActive"
    );
}

#[test]
fn demand_active_captures_server_input_flags() {
    use ironrdp_pdu::rdp::capability_sets::InputFlags;

    let config = test_config();
    let mut seq = ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID);
    let mut output = WriteBuf::new();

    let frame = encode_server_share_control(ShareControlPdu::ServerDemandActive(SERVER_DEMAND_ACTIVE.clone()));
    seq.step(&frame, None, &mut output).unwrap();

    match seq.connection_activation_state() {
        ConnectionActivationState::ConnectionFinalization { input_flags, .. } => {
            assert_eq!(
                input_flags,
                InputFlags::SCANCODES | InputFlags::MOUSEX | InputFlags::UNICODE | InputFlags::FASTPATH_INPUT_2,
                "input_flags should mirror the Input capability in the Server Demand Active"
            );
        }
        other => panic!("expected ConnectionFinalization, got: {other:?}"),
    }
}

#[test]
fn demand_active_without_input_capability_yields_empty_input_flags() {
    use ironrdp_pdu::rdp::capability_sets::{CapabilitySet, InputFlags};

    let config = test_config();
    let mut seq = ConnectionActivationSequence::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID);
    let mut output = WriteBuf::new();

    let mut demand_active = SERVER_DEMAND_ACTIVE.clone();
    demand_active
        .pdu
        .capability_sets
        .retain(|c| !matches!(c, CapabilitySet::Input(_)));

    let frame = encode_server_share_control(ShareControlPdu::ServerDemandActive(demand_active));
    seq.step(&frame, None, &mut output).unwrap();

    match seq.connection_activation_state() {
        ConnectionActivationState::ConnectionFinalization { input_flags, .. } => {
            assert_eq!(input_flags, InputFlags::empty());
        }
        other => panic!("expected ConnectionFinalization, got: {other:?}"),
    }
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

/// A connector parked in `MultitransportPending` with one request outstanding.
///
/// `soft_sync` mirrors what the connector would have derived from both peers'
/// GCC `MultiTransportChannelData` flags, and decides whether the response paths
/// are allowed to put an Initiate Multitransport Response on the message channel.
fn multitransport_pending_connector(soft_sync: bool, request: MultitransportRequestPdu) -> ClientConnector {
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::MultitransportPending {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
        message_channel_id: Some(MESSAGE_CHANNEL_ID),
        request,
        requests_seen: 1,
        soft_sync,
    };
    connector
}

/// Encode an Initiate Multitransport Request as a server-to-client
/// SendDataIndication on the given channel.
fn encode_server_multitransport_request(request: &MultitransportRequestPdu, channel_id: u16) -> Vec<u8> {
    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id,
        user_data: Cow::Owned(encode_vec(request).unwrap()),
    });

    encode_vec(&X224(indication)).unwrap()
}

/// A connector parked in `MultitransportBootstrapping`, as it would be straight
/// out of licensing.
fn multitransport_bootstrapping_connector() -> ClientConnector {
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::MultitransportBootstrapping {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
        message_channel_id: Some(MESSAGE_CHANNEL_ID),
        requests_seen: 0,
    };
    connector
}

#[test]
fn multitransport_request_is_surfaced_without_waiting_for_another_pdu() {
    // The one that matters: MS-RDPBCGR 3.2.5.15.1 requires the client to act on
    // a request as soon as it decodes it. A server may send a single request and
    // then wait for the client to bring UDP up before sending Demand Active, so
    // a connector that holds the request until some later PDU arrives deadlocks
    // against a server that is itself waiting.
    let mut connector = multitransport_bootstrapping_connector();
    let frame = encode_server_multitransport_request(
        &multitransport_request(1, RequestedProtocol::UdpFecR),
        MESSAGE_CHANNEL_ID,
    );
    let mut output = WriteBuf::new();

    connector.step(&frame, None, &mut output).unwrap();

    assert!(
        connector.should_perform_multitransport(),
        "the request must be surfaced on arrival, not held pending a following PDU"
    );
    assert_eq!(connector.multitransport_request().unwrap().request_id, 1);
}

#[test]
fn active_processor_surfaces_multitransport_request_on_message_channel() {
    let request = multitransport_request(42, RequestedProtocol::UdpFecR);
    let frame = encode_server_multitransport_request(&request, MESSAGE_CHANNEL_ID);
    let mut processor = Processor::new(
        StaticChannelSet::new(),
        USER_CHANNEL_ID,
        IO_CHANNEL_ID,
        Some(MESSAGE_CHANNEL_ID),
        SHARE_ID,
    );

    let outputs = processor
        .process(&frame, &mut None)
        .expect("active processor should surface multitransport request");

    assert!(matches!(
        outputs.as_slice(),
        [ProcessorOutput::MultitransportRequest(decoded)] if decoded == &request
    ));
}

#[test]
fn responding_returns_to_bootstrapping_for_the_next_request() {
    // Two requests are permitted, and the second only arrives after the first
    // has been answered, so the connector has to go back to reading rather than
    // straight on to capabilities exchange.
    let mut connector = multitransport_bootstrapping_connector();
    let mut output = WriteBuf::new();

    for request_id in 1..=2 {
        let frame = encode_server_multitransport_request(
            &multitransport_request(request_id, RequestedProtocol::UdpFecR),
            MESSAGE_CHANNEL_ID,
        );
        connector.step(&frame, None, &mut output).unwrap();
        assert!(connector.should_perform_multitransport());
        assert_eq!(connector.multitransport_request().unwrap().request_id, request_id);

        connector
            .complete_multitransport(MultitransportResult::Success, &mut output)
            .unwrap();
    }

    assert!(
        matches!(
            connector.state,
            ClientConnectorState::MultitransportBootstrapping { requests_seen: 2, .. }
        ),
        "after the second response the connector must still be reading, with the cap tracked"
    );
}

#[test]
fn third_multitransport_request_is_rejected() {
    // MS-RDPBCGR 2.2.15.1 caps the set at two, one per transport protocol.
    let mut connector = multitransport_bootstrapping_connector();
    let mut output = WriteBuf::new();

    for request_id in 1..=2 {
        let frame = encode_server_multitransport_request(
            &multitransport_request(request_id, RequestedProtocol::UdpFecR),
            MESSAGE_CHANNEL_ID,
        );
        connector.step(&frame, None, &mut output).unwrap();
        connector
            .complete_multitransport(MultitransportResult::Success, &mut output)
            .unwrap();
    }

    let frame = encode_server_multitransport_request(
        &multitransport_request(3, RequestedProtocol::UdpFecR),
        MESSAGE_CHANNEL_ID,
    );
    assert!(connector.step(&frame, None, &mut output).is_err());
}

#[test]
fn demand_active_on_the_io_channel_ends_bootstrapping() {
    // The I/O channel is no longer speculatively decoded as multitransport; a
    // PDU arriving there is the Demand Active and ends the phase.
    let mut connector = multitransport_bootstrapping_connector();
    let frame = encode_server_share_control(ShareControlPdu::ServerDemandActive(SERVER_DEMAND_ACTIVE.clone()));
    let mut output = WriteBuf::new();

    connector.step(&frame, None, &mut output).unwrap();

    assert!(
        matches!(connector.state, ClientConnectorState::ConnectionFinalization { .. }),
        "a Demand Active must take the connector out of bootstrapping"
    );
    assert!(!connector.should_perform_multitransport());
}

#[test]
fn should_perform_multitransport_reflects_pending_state() {
    let connector = multitransport_pending_connector(true, multitransport_request(1, RequestedProtocol::UdpFecR));
    assert!(connector.should_perform_multitransport());
    assert_eq!(connector.multitransport_request().unwrap().request_id, 1);
}

#[test]
fn complete_multitransport_responds_on_the_message_channel() {
    // MS-RDPBCGR 2.2.15.2 and 3.2.5.15.2 put the Initiate Multitransport
    // Response on the negotiated MCS message channel. Sending it on the I/O
    // channel means a Soft-Sync server never sees it, and since both channels
    // are valid MCS targets nothing downstream would notice.
    let mut connector = multitransport_pending_connector(true, multitransport_request(1, RequestedProtocol::UdpFecR));
    let mut output = WriteBuf::new();

    connector
        .complete_multitransport(MultitransportResult::Success, &mut output)
        .unwrap();

    let X224(McsMessage::SendDataRequest(request)) = decode(output.filled()).unwrap() else {
        panic!("the multitransport response must be written");
    };

    assert_eq!(
        request.channel_id, MESSAGE_CHANNEL_ID,
        "the response must target the MCS message channel, not the I/O channel"
    );

    let response: MultitransportResponsePdu = decode(&request.user_data).unwrap();
    assert_eq!(response.hr_response, MultitransportResponsePdu::S_OK);
}

#[test]
fn complete_multitransport_carries_failure_results() {
    let mut connector = multitransport_pending_connector(true, multitransport_request(7, RequestedProtocol::UdpFecR));
    let mut output = WriteBuf::new();

    connector
        .complete_multitransport(MultitransportResult::Failure(0x8000_0001), &mut output)
        .unwrap();

    let X224(McsMessage::SendDataRequest(request)) = decode(output.filled()).unwrap() else {
        panic!("the multitransport response must be written");
    };
    let response: MultitransportResponsePdu = decode(&request.user_data).unwrap();

    assert_eq!(response.request_id, 7);
    assert_eq!(response.hr_response, 0x8000_0001);
}

#[test]
fn complete_multitransport_omits_success_without_soft_sync() {
    // MS-RDPBCGR 2.2.15.2: `S_OK` "MUST only be sent to a server that advertises
    // the SOFTSYNC_TCP_TO_UDP flag". Success is therefore the one outcome that
    // has to stay off the wire here; the failure case below still reports.
    let mut connector = multitransport_pending_connector(false, multitransport_request(1, RequestedProtocol::UdpFecR));
    let mut output = WriteBuf::new();

    connector
        .complete_multitransport(MultitransportResult::Success, &mut output)
        .unwrap();

    assert!(
        output.filled().is_empty(),
        "S_OK may not be emitted when Soft-Sync was not negotiated"
    );
    assert!(matches!(
        connector.state,
        ClientConnectorState::MultitransportBootstrapping { .. }
    ));
}

#[test]
fn skip_multitransport_declines_with_e_abort_under_soft_sync() {
    // MS-RDPBCGR 3.2.5.15.1 requires a response to every request once Soft-Sync
    // is mutually negotiated, whatever the outcome. Both the async and blocking
    // drivers skip automatically, so a silent skip leaves a compliant server
    // waiting on a response it is entitled to.
    let mut connector = multitransport_pending_connector(true, multitransport_request(1, RequestedProtocol::UdpFecR));
    let mut output = WriteBuf::new();

    connector.skip_multitransport(&mut output).unwrap();

    let X224(McsMessage::SendDataRequest(request)) = decode(output.filled()).unwrap() else {
        panic!("a declined multitransport must still put a response on the wire");
    };
    assert_eq!(request.channel_id, MESSAGE_CHANNEL_ID);

    let response: MultitransportResponsePdu = decode(&request.user_data).unwrap();
    assert_eq!(
        response.hr_response,
        MultitransportResponsePdu::E_ABORT,
        "declining must report E_ABORT, not success"
    );
    assert_eq!(response.request_id, 1);
}

#[test]
fn skip_multitransport_reports_e_abort_without_soft_sync() {
    // MS-RDPBCGR 3.2.5.15.1 asks for the response whenever the client could not
    // initiate the sideband channel, with no Soft-Sync condition attached, and
    // only `S_OK` is restricted by 2.2.15.2. Declining is a failure, so it is
    // reported either way.
    let mut connector = multitransport_pending_connector(false, multitransport_request(1, RequestedProtocol::UdpFecR));
    let mut output = WriteBuf::new();

    connector.skip_multitransport(&mut output).unwrap();

    let X224(McsMessage::SendDataRequest(request)) = decode(output.filled()).unwrap() else {
        panic!("declining must put a failure response on the wire");
    };
    assert_eq!(request.channel_id, MESSAGE_CHANNEL_ID);

    let response: MultitransportResponsePdu = decode(&request.user_data).unwrap();
    assert_eq!(response.request_id, 1);
    assert_eq!(
        response.hr_response,
        MultitransportResponsePdu::E_ABORT,
        "the specified failure report must not be dropped just because Soft-Sync is absent"
    );
}

#[test]
fn failing_to_respond_leaves_the_connector_able_to_act() {
    // A Soft-Sync session with no message channel is contradictory and the
    // response cannot be sent. The error must not also destroy the state: a
    // connector left `Consumed` gives the caller nothing to inspect, no way to
    // retry, and no way to decline.
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::MultitransportPending {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
        message_channel_id: None,
        request: multitransport_request(1, RequestedProtocol::UdpFecR),
        requests_seen: 1,
        soft_sync: true,
    };
    let mut output = WriteBuf::new();

    assert!(
        connector
            .complete_multitransport(MultitransportResult::Success, &mut output)
            .is_err()
    );

    assert!(
        connector.should_perform_multitransport(),
        "the connector must still be in MultitransportPending after a failed response"
    );
    assert_eq!(
        connector.multitransport_request().unwrap().request_id,
        1,
        "the request must survive so the caller can see what failed"
    );

    // And the failure is reported the same way a second time rather than
    // degenerating into an outside-state error.
    assert!(connector.skip_multitransport(&mut output).is_err());
    assert!(connector.should_perform_multitransport());
}

#[test]
fn complete_multitransport_outside_pending_state_errors() {
    let mut connector = ClientConnector::new(test_config(), "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::CapabilitiesExchange {
        connection_activation: ConnectionActivationSequence::new(test_config(), IO_CHANNEL_ID, USER_CHANNEL_ID),
    };
    let mut output = WriteBuf::new();

    assert!(
        connector
            .complete_multitransport(MultitransportResult::Success, &mut output)
            .is_err()
    );

    // The error must not cost the caller the state it was in. Reporting
    // "you called this from the wrong state" while destroying that state
    // leaves nothing to recover to.
    assert!(
        matches!(connector.state, ClientConnectorState::CapabilitiesExchange { .. }),
        "the connector must still be in CapabilitiesExchange after the outside-state error"
    );
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
