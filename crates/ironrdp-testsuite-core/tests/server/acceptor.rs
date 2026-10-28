use core::cell::RefCell;
use core::future::{Ready, ready};
use std::borrow::Cow;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;

use ironrdp_acceptor::{
    Acceptor, ChannelConnectionSequence, ConnectionSetupHandler, FinalizationSequence, MultitransportSecurityRng,
    ReceivedCredentials, accept_begin, accept_credssp_with, accept_finalize, accept_finalize_with,
};
use ironrdp_async::bytes::BytesMut;
use ironrdp_async::{Framed, FramedRead, FramedWrite, NetworkClient, StreamWrapper};
use ironrdp_connector::credssp::CredsspSequence;
use ironrdp_connector::sspi::credssp::{ClientMode, ClientState, CredSspClient, CredSspMode, NStatusCode, TsRequest};
use ironrdp_connector::sspi::generator::{GeneratorState, NetworkRequest};
use ironrdp_connector::sspi::ntlm::NtlmConfig;
use ironrdp_connector::sspi::{AuthIdentity, NegotiateConfig, Username};
use ironrdp_connector::{
    ConnectorError, ConnectorErrorExt as _, ConnectorErrorKind, ConnectorResult, DesktopSize, Sequence, Written,
    encode_x224_packet,
};
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_dvc::DrdynvcServer;
use ironrdp_dvc::pdu::{CreateResponsePdu, CreationStatus, DrdynvcClientPdu};
use ironrdp_echo::server::EchoServer;
use ironrdp_pdu::gcc::{
    ClientCoreOptionalData, ClientEarlyCapabilityFlags, ClientMessageChannelData, ClientMonitorData, Monitor,
    MonitorFlags, MultiTransportChannelData, MultiTransportFlags,
};
use ironrdp_pdu::mcs::{self, ConnectInitial};
use ironrdp_pdu::nego::{self, SecurityProtocol};
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::rdp::capability_sets::{CapabilitySet, Rail, RailSupportLevel, WindowList, WindowSupportLevel};
use ironrdp_pdu::rdp::client_info::{ClientInfoFlags, CompressionType, Credentials};
use ironrdp_pdu::rdp::finalization_messages::{ControlAction, ControlPdu, SynchronizePdu};
use ironrdp_pdu::rdp::headers::{
    BasicSecurityHeader, BasicSecurityHeaderFlags, CompressionFlags, ShareControlHeader, ShareControlPdu,
    ShareDataHeader, ShareDataPdu, StreamPriority, decode_share_control,
};
use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, MultitransportResponsePdu, RequestedProtocol};
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu};
use ironrdp_pdu::x224::{X224, X224Data};
use ironrdp_svc::{StaticVirtualChannel, SvcMessage, SvcProcessor as _, client_encode_svc_messages};
use ironrdp_testsuite_core::client_info::CLIENT_INFO_UNICODE;
use ironrdp_testsuite_core::gcc::CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS;
use ironrdp_testsuite_core::rdp::{CLIENT_DEMAND_ACTIVE_PDU_BUFFER, CLIENT_INFO_PDU_BUFFER};

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;

/// Build a minimal ConnectionRequest with the given protocols and encode it.
fn encode_connection_request(protocol: SecurityProtocol) -> Vec<u8> {
    let request = nego::ConnectionRequest {
        nego_data: None,
        flags: nego::RequestFlags::empty(),
        protocol,
        correlation_info: None,
    };
    let mut buf = WriteBuf::new();
    ironrdp_core::encode_buf(&X224(request), &mut buf).unwrap();
    buf.filled().to_vec()
}

/// When server requires TLS but client only offers HYBRID|HYBRID_EX,
/// the acceptor must write an RDP_NEG_FAILURE PDU and return an error.
#[test]
fn neg_failure_on_protocol_mismatch() {
    let mut acceptor = Acceptor::new(
        SecurityProtocol::SSL,
        DesktopSize {
            width: 1920,
            height: 1080,
        },
        Vec::new(),
        None,
    );

    // Step 1: feed the connection request (HYBRID | HYBRID_EX, no SSL)
    let request_bytes = encode_connection_request(SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX);
    let mut output = WriteBuf::new();
    let written = acceptor.step(&request_bytes, None, &mut output).unwrap();
    assert!(matches!(written, Written::Nothing));

    // Step 2: acceptor tries to send confirm, finds no common protocol
    let mut output = WriteBuf::new();
    let result = acceptor.step(&[], None, &mut output);

    // Must be an error
    assert!(result.is_err(), "expected error on protocol mismatch");

    // Must have written an RDP_NEG_FAILURE PDU to the output buffer
    let response_bytes = output.filled();
    assert!(!response_bytes.is_empty(), "expected RDP_NEG_FAILURE PDU in output");

    // Decode the response and verify it's a Failure with the right code
    let confirm = decode::<X224<nego::ConnectionConfirm>>(response_bytes).unwrap().0;
    match confirm {
        nego::ConnectionConfirm::Failure { code } => {
            assert_eq!(code, nego::FailureCode::SSL_REQUIRED_BY_SERVER);
        }
        nego::ConnectionConfirm::Response { .. } => {
            panic!("expected Failure, got Response");
        }
    }
}

/// When server and client agree on SSL, negotiation succeeds normally.
#[test]
fn neg_success_when_protocols_match() {
    let mut acceptor = Acceptor::new(
        SecurityProtocol::SSL,
        DesktopSize {
            width: 1920,
            height: 1080,
        },
        Vec::new(),
        None,
    );

    let request_bytes = encode_connection_request(SecurityProtocol::SSL | SecurityProtocol::HYBRID);
    let mut output = WriteBuf::new();
    acceptor.step(&request_bytes, None, &mut output).unwrap();

    let mut output = WriteBuf::new();
    let written = acceptor.step(&[], None, &mut output).unwrap();
    assert!(!matches!(written, Written::Nothing));

    let response_bytes = output.filled();
    let confirm = decode::<X224<nego::ConnectionConfirm>>(response_bytes).unwrap().0;
    match confirm {
        nego::ConnectionConfirm::Response { protocol, flags } => {
            assert_eq!(protocol, SecurityProtocol::SSL);
            // The acceptor advertises support for Extended Client Data Blocks so the
            // client sends its Client Message Channel Data, enabling the message
            // channel to be negotiated.
            assert!(flags.contains(nego::ResponseFlags::EXTENDED_CLIENT_DATA_SUPPORTED));
        }
        nego::ConnectionConfirm::Failure { .. } => {
            panic!("expected Response, got Failure");
        }
    }
}

/// When the client advertises the message channel (Client Message Channel Data),
/// the acceptor allocates an MCS channel ID for it and returns it in Server
/// Message Channel Data. The ID is allocated after the I/O channel and any
/// static virtual channels, so the expected value is derived from the server's
/// network block rather than hard-coded.
#[test]
fn message_channel_advertised_when_client_requests_it() {
    let mut acceptor = Acceptor::new(
        SecurityProtocol::SSL,
        DesktopSize {
            width: 1920,
            height: 1080,
        },
        Vec::new(),
        None,
    );

    // Connection request -> confirm -> (TLS upgrade) -> ready for ConnectInitial.
    let request_bytes = encode_connection_request(SecurityProtocol::SSL);
    acceptor.step(&request_bytes, None, &mut WriteBuf::new()).unwrap();
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    acceptor.mark_security_upgrade_as_done();

    // Client GCC with the message channel block and no network channels, so the
    // allocated ID is deterministic.
    let mut blocks = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    blocks.network = None;
    blocks.message_channel = Some(ClientMessageChannelData);
    let connect_initial = ConnectInitial::with_gcc_blocks(blocks).unwrap();
    let mut initial_buf = WriteBuf::new();
    encode_x224_packet(&connect_initial, &mut initial_buf).unwrap();

    acceptor.step(initial_buf.filled(), None, &mut WriteBuf::new()).unwrap();

    let mut output = WriteBuf::new();
    acceptor.step(&[], None, &mut output).unwrap();

    let payload = decode::<X224<X224Data<'_>>>(output.filled()).unwrap().0;
    let response = decode::<mcs::ConnectResponse>(payload.data.as_ref()).unwrap();
    let server_blocks = response.conference_create_response.gcc_blocks();

    let message_channel = server_blocks
        .message_channel
        .as_ref()
        .expect("acceptor must advertise Server Message Channel Data");

    // The message channel is allocated after the I/O channel and any static
    // virtual channels, so derive the expected ID from the server's network
    // block instead of coupling the assertion to the I/O channel base.
    let network = &server_blocks.network;
    let channel_count = u16::try_from(network.channel_ids.len()).expect("channel count fits in u16");
    let expected = network.io_channel + channel_count + 1;
    assert_eq!(message_channel.mcs_message_channel_id, expected);
}

/// When server requires HYBRID but client only offers SSL, the failure code
/// should be HYBRID_REQUIRED_BY_SERVER.
#[test]
fn neg_failure_hybrid_required() {
    let mut acceptor = Acceptor::new(
        SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX,
        DesktopSize {
            width: 1920,
            height: 1080,
        },
        Vec::new(),
        None,
    );

    let request_bytes = encode_connection_request(SecurityProtocol::SSL);
    let mut output = WriteBuf::new();
    acceptor.step(&request_bytes, None, &mut output).unwrap();

    let mut output = WriteBuf::new();
    let result = acceptor.step(&[], None, &mut output);
    assert!(result.is_err());

    let response_bytes = output.filled();
    let confirm = decode::<X224<nego::ConnectionConfirm>>(response_bytes).unwrap().0;
    match confirm {
        nego::ConnectionConfirm::Failure { code } => {
            assert_eq!(code, nego::FailureCode::HYBRID_REQUIRED_BY_SERVER);
        }
        nego::ConnectionConfirm::Response { .. } => {
            panic!("expected Failure, got Response");
        }
    }
}

pub(super) fn encode_send_data_request(initiator_id: u16, channel_id: u16, user_data: &[u8]) -> Vec<u8> {
    let mut buf = WriteBuf::new();
    ironrdp_core::encode_buf(
        &X224(mcs::SendDataRequest {
            initiator_id,
            channel_id,
            user_data: Cow::Borrowed(user_data),
        }),
        &mut buf,
    )
    .unwrap();
    buf.filled().to_vec()
}

/// Encode a client finalization ShareData PDU (Synchronize, Control, FontList),
/// mirroring the shape `ironrdp_acceptor`'s own `wrap_share_data` uses for the
/// server's confirmations of the same messages.
fn encode_client_share_data(pdu: ShareDataPdu) -> Vec<u8> {
    let header = ShareControlHeader {
        share_id: 0,
        pdu_source: 0,
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: pdu,
            stream_priority: StreamPriority::Undefined,
            compression_flags: CompressionFlags::empty(),
            compression_type: CompressionType::K8,
        }),
    };
    encode_vec(&header).unwrap()
}

/// Drives `acceptor` from a fresh `InitiationWaitRequest` through
/// `SecureSettingsExchange`, i.e. everything that precedes licensing and is
/// identical regardless of what the multitransport tests below want to
/// exercise: negotiation, TLS upgrade marker, GCC exchange (with the given
/// client blocks) and the full MCS channel join sequence, then the Client
/// Info PDU. Returns `(user_channel_id, io_channel_id, message_channel_id)`
/// as actually assigned by the acceptor, so callers never have to hard-code
/// them.
fn drive_to_secure_settings_exchange(
    acceptor: &mut Acceptor,
    client_blocks: ironrdp_pdu::gcc::ClientGccBlocks,
) -> (u16, u16, Option<u16>, Option<MultiTransportFlags>) {
    drive_to_secure_settings_exchange_with_info(acceptor, client_blocks, &CLIENT_INFO_PDU_BUFFER)
}

fn drive_to_secure_settings_exchange_with_info(
    acceptor: &mut Acceptor,
    client_blocks: ironrdp_pdu::gcc::ClientGccBlocks,
    client_info: &[u8],
) -> (u16, u16, Option<u16>, Option<MultiTransportFlags>) {
    let request_bytes = encode_connection_request(SecurityProtocol::SSL);
    acceptor.step(&request_bytes, None, &mut WriteBuf::new()).unwrap();
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    acceptor.mark_security_upgrade_as_done();

    let connect_initial = ConnectInitial::with_gcc_blocks(client_blocks).unwrap();
    let mut initial_buf = WriteBuf::new();
    encode_x224_packet(&connect_initial, &mut initial_buf).unwrap();
    acceptor.step(initial_buf.filled(), None, &mut WriteBuf::new()).unwrap();

    let mut output = WriteBuf::new();
    acceptor.step(&[], None, &mut output).unwrap();
    let payload = decode::<X224<X224Data<'_>>>(output.filled()).unwrap().0;
    let response = decode::<mcs::ConnectResponse>(payload.data.as_ref()).unwrap();
    let server_blocks = response.conference_create_response.gcc_blocks();
    let io_channel_id = server_blocks.network.io_channel;
    let static_channel_ids = server_blocks.network.channel_ids.clone();
    let message_channel_id = server_blocks.message_channel.as_ref().map(|m| m.mcs_message_channel_id);
    let server_multitransport = server_blocks.multi_transport_channel.as_ref().map(|m| m.flags);

    // Erect Domain Request, Attach User Request, then confirm.
    let mut buf = WriteBuf::new();
    ironrdp_core::encode_buf(
        &X224(mcs::ErectDomainPdu {
            sub_height: 0,
            sub_interval: 0,
        }),
        &mut buf,
    )
    .unwrap();
    acceptor.step(buf.filled(), None, &mut WriteBuf::new()).unwrap();

    let mut buf = WriteBuf::new();
    ironrdp_core::encode_buf(&X224(mcs::AttachUserRequest), &mut buf).unwrap();
    acceptor.step(buf.filled(), None, &mut WriteBuf::new()).unwrap();

    let mut output = WriteBuf::new();
    acceptor.step(&[], None, &mut output).unwrap();
    let attach_user_confirm = decode::<X224<mcs::AttachUserConfirm>>(output.filled()).unwrap().0;
    let user_channel_id = attach_user_confirm.initiator_id;

    let mut to_join = vec![user_channel_id, io_channel_id];
    to_join.extend(static_channel_ids);
    to_join.extend(message_channel_id);
    for channel_id in to_join {
        let mut buf = WriteBuf::new();
        ironrdp_core::encode_buf(
            &X224(mcs::ChannelJoinRequest {
                initiator_id: user_channel_id,
                channel_id,
            }),
            &mut buf,
        )
        .unwrap();
        acceptor.step(buf.filled(), None, &mut WriteBuf::new()).unwrap();
        acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    }

    // RdpSecurityCommencement (no input) -> SecureSettingsExchange, then the
    // Client Info PDU on the I/O channel.
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    let client_info = encode_send_data_request(user_channel_id, io_channel_id, client_info);
    acceptor.step(&client_info, None, &mut WriteBuf::new()).unwrap();

    (
        user_channel_id,
        io_channel_id,
        message_channel_id,
        server_multitransport,
    )
}

fn rail_acceptor_waiting_confirm() -> (Acceptor, u16, u16) {
    let mut acceptor = Acceptor::new(
        SecurityProtocol::SSL,
        DesktopSize {
            width: 1024,
            height: 768,
        },
        vec![
            CapabilitySet::Rail(Rail {
                support_level: RailSupportLevel::SUPPORTED,
            }),
            CapabilitySet::WindowList(WindowList {
                support_level: WindowSupportLevel::SupportedEx,
                num_icon_caches: 3,
                num_icon_cache_entries: 12,
            }),
        ],
        None,
    );
    let mut info = decode::<ClientInfoPdu>(&CLIENT_INFO_PDU_BUFFER).unwrap();
    info.client_info.flags.insert(ClientInfoFlags::RAIL);
    let mut gcc = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    gcc.network = None;
    let (user_channel_id, io_channel_id, _, _) =
        drive_to_secure_settings_exchange_with_info(&mut acceptor, gcc, &encode_vec(&info).unwrap());
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    (acceptor, user_channel_id, io_channel_id)
}

fn rail_confirm(capabilities: Vec<CapabilitySet>, user_channel_id: u16, io_channel_id: u16) -> Vec<u8> {
    let mut confirm = decode::<ShareControlHeader>(&CLIENT_DEMAND_ACTIVE_PDU_BUFFER).unwrap();
    let ShareControlPdu::ClientConfirmActive(active) = &mut confirm.share_control_pdu else {
        panic!("fixture must contain Confirm Active");
    };
    active.pdu.capability_sets = capabilities;
    encode_send_data_request(user_channel_id, io_channel_id, &encode_vec(&confirm).unwrap())
}

#[test]
fn rail_confirmation_requires_window_list_capability() {
    // oracle: spec MS-RDPERP §3.3.5.1.5: an agreed RAIL connection requires both client capability sets.
    let (mut acceptor, user_channel_id, io_channel_id) = rail_acceptor_waiting_confirm();
    let capabilities = vec![CapabilitySet::Rail(Rail {
        support_level: RailSupportLevel::SUPPORTED,
    })];
    let result = acceptor.step(
        &rail_confirm(capabilities, user_channel_id, io_channel_id),
        None,
        &mut WriteBuf::new(),
    );

    assert!(result.is_err(), "RAIL must reject a client without Window List support");
}

#[test]
fn rail_confirmation_disables_icon_caches_larger_than_the_server_offer() {
    // oracle: reference MS-RDPERP §3.3.5.1.5 and §6 note 36: Windows disables icon caches exceeding the server offer.
    let (mut acceptor, user_channel_id, io_channel_id) = rail_acceptor_waiting_confirm();
    let capabilities = vec![
        CapabilitySet::Rail(Rail {
            support_level: RailSupportLevel::SUPPORTED,
        }),
        CapabilitySet::WindowList(WindowList {
            support_level: WindowSupportLevel::SupportedEx,
            num_icon_caches: 4,
            num_icon_cache_entries: 13,
        }),
    ];
    acceptor
        .step(
            &rail_confirm(capabilities, user_channel_id, io_channel_id),
            None,
            &mut WriteBuf::new(),
        )
        .unwrap();
    let remote_app = acceptor.remote_app_capabilities().expect("RAIL should be negotiated");
    assert_eq!(remote_app.window_list.num_icon_caches, 0);
    assert_eq!(remote_app.window_list.num_icon_cache_entries, 0);
}

// oracle: spec MS-RDPERP §3.3.5.1.5: Rail must be present and both support levels must be nonzero.
#[rstest::rstest]
#[case::missing_rail(None, WindowSupportLevel::SupportedEx)]
#[case::rail_disabled(Some(RailSupportLevel::empty()), WindowSupportLevel::SupportedEx)]
#[case::window_disabled(Some(RailSupportLevel::SUPPORTED), WindowSupportLevel::NotSupported)]
fn rail_confirmation_rejects_missing_or_disabled_support(
    #[case] rail: Option<RailSupportLevel>,
    #[case] window: WindowSupportLevel,
) {
    let (mut acceptor, user, io) = rail_acceptor_waiting_confirm();
    let capabilities = rail
        .map(|support_level| CapabilitySet::Rail(Rail { support_level }))
        .into_iter()
        .chain(Some(CapabilitySet::WindowList(WindowList {
            support_level: window,
            num_icon_caches: 3,
            num_icon_cache_entries: 12,
        })))
        .collect();
    assert!(
        acceptor
            .step(&rail_confirm(capabilities, user, io), None, &mut WriteBuf::new())
            .is_err()
    );
}

// oracle: spec MS-RDPERP §3.3.5.1.5: client icon-cache dimensions can be at or below the server offer.
#[rstest::rstest]
#[case::smaller(2, 8)]
#[case::equal(3, 12)]
#[case::disabled(0, 0)]
fn rail_confirmation_preserves_supported_icon_cache_limits(#[case] caches: u8, #[case] entries: u16) {
    let (mut acceptor, user, io) = rail_acceptor_waiting_confirm();
    let capabilities = vec![
        CapabilitySet::Rail(Rail {
            support_level: RailSupportLevel::SUPPORTED,
        }),
        CapabilitySet::WindowList(WindowList {
            support_level: WindowSupportLevel::Supported,
            num_icon_caches: caches,
            num_icon_cache_entries: entries,
        }),
    ];
    acceptor
        .step(&rail_confirm(capabilities, user, io), None, &mut WriteBuf::new())
        .unwrap();
    let effective = &acceptor.remote_app_capabilities().unwrap().window_list;
    assert_eq!(effective.num_icon_caches, caches);
    assert_eq!(effective.num_icon_cache_entries, entries);
    assert_eq!(effective.support_level, WindowSupportLevel::Supported);
}

// oracle: reference MS-RDPERP §3.3.5.1.5, §6 product behavior note 36: either excessive dimension disables both caches.
#[rstest::rstest]
#[case::too_many_caches(4, 12)]
#[case::too_many_entries(3, 13)]
fn rail_confirmation_checks_each_icon_cache_dimension(#[case] caches: u8, #[case] entries: u16) {
    let (mut acceptor, user, io) = rail_acceptor_waiting_confirm();
    let capabilities = vec![
        CapabilitySet::Rail(Rail {
            support_level: RailSupportLevel::SUPPORTED,
        }),
        CapabilitySet::WindowList(WindowList {
            support_level: WindowSupportLevel::SupportedEx,
            num_icon_caches: caches,
            num_icon_cache_entries: entries,
        }),
    ];
    acceptor
        .step(&rail_confirm(capabilities, user, io), None, &mut WriteBuf::new())
        .unwrap();
    let effective = &acceptor.remote_app_capabilities().unwrap().window_list;
    assert_eq!(effective.num_icon_caches, 0);
    assert_eq!(effective.num_icon_cache_entries, 0);
}

// oracle: spec MS-RDPBCGR §1.3.1.3 and MS-RDPERP §3.3.5.1.5: reactivation rechecks capabilities without resending Client Info.
#[test]
fn rail_reactivation_preserves_client_info_and_reconfirms_window_support() {
    let (mut acceptor, user, io) = rail_acceptor_waiting_confirm();
    let capabilities = vec![
        CapabilitySet::Rail(Rail {
            support_level: RailSupportLevel::SUPPORTED,
        }),
        CapabilitySet::WindowList(WindowList {
            support_level: WindowSupportLevel::SupportedEx,
            num_icon_caches: 3,
            num_icon_cache_entries: 12,
        }),
    ];
    let confirm = rail_confirm(capabilities, user, io);
    acceptor.step(&confirm, None, &mut WriteBuf::new()).unwrap();
    complete_finalization(&mut acceptor, user, io);
    let result = acceptor.get_result().unwrap();
    let mut acceptor = Acceptor::new_deactivation_reactivation(
        acceptor,
        result.static_channels,
        DesktopSize {
            width: 1280,
            height: 720,
        },
    )
    .unwrap();
    assert!(acceptor.client_info_flags().unwrap().contains(ClientInfoFlags::RAIL));
    assert!(acceptor.remote_app_capabilities().is_none());
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    acceptor.step(&confirm, None, &mut WriteBuf::new()).unwrap();
    complete_finalization(&mut acceptor, user, io);
    assert_eq!(
        acceptor.remote_app_capabilities().unwrap().window_list.num_icon_caches,
        3
    );
}

pub(super) fn client_gcc_with_message_channel_and_multitransport(
    offer: Option<MultiTransportFlags>,
) -> ironrdp_pdu::gcc::ClientGccBlocks {
    let mut blocks = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    blocks.network = None;
    blocks.message_channel = Some(ClientMessageChannelData);
    blocks.multi_transport_channel = offer.map(|flags| MultiTransportChannelData { flags });
    blocks
}

/// Builds a fresh `Acceptor` at a common 1920x1080 desktop size with no static
/// channels or credentials. `offer` is passed straight through to
/// `set_multitransport_offer`; pass `None` to exercise the default-disabled
/// path, which is also what tests unrelated to multitransport use.
fn multitransport_acceptor(offer: Option<MultiTransportFlags>) -> Acceptor {
    let mut acceptor = Acceptor::new(
        SecurityProtocol::SSL,
        DesktopSize {
            width: 1920,
            height: 1080,
        },
        Vec::new(),
        None,
    );
    acceptor.set_multitransport_offer(offer);
    acceptor
}

/// The full happy path: the acceptor offers reliable UDP multitransport, the
/// client reciprocates, so the request goes out on the message channel and
/// `multitransport_request()` surfaces it. A late Initiate Multitransport
/// Response then arrives, interleaved before the mandatory Confirm Active
/// exactly as a real client would send it (MS-RDPBCGR 3.2.5.15.1: sent while
/// resolving its own bootstrapping, strictly before it ever reads Demand
/// Active), and the acceptor must tolerate it rather than erroring out while
/// still reaching capabilities confirmation.
#[test]
fn multitransport_offered_and_client_reciprocates() {
    let mut acceptor = multitransport_acceptor(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));

    let client_blocks =
        client_gcc_with_message_channel_and_multitransport(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
    let (user_channel_id, _io_channel_id, message_channel_id, server_multitransport) =
        drive_to_secure_settings_exchange(&mut acceptor, client_blocks);
    let message_channel_id = message_channel_id.expect("message channel negotiated");
    assert_eq!(
        server_multitransport,
        Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR),
        "server should advertise the offer once the client reciprocated"
    );

    // LicensingExchange (sends license) -> MultitransportBootstrapping.
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();

    assert!(
        acceptor.multitransport_request().is_none(),
        "no request sent until MultitransportBootstrapping actually runs"
    );

    // MultitransportBootstrapping: decides to offer, sends the request.
    let mut output = WriteBuf::new();
    let written = acceptor.step(&[], None, &mut output).unwrap();
    assert!(!matches!(written, Written::Nothing), "expected the request to be sent");

    let sent_request = acceptor
        .multitransport_request()
        .expect("request recorded after MultitransportBootstrapping")
        .clone();
    assert_eq!(sent_request.requested_protocol, RequestedProtocol::UdpFecR);
    assert_eq!(
        sent_request.security_header.flags,
        BasicSecurityHeaderFlags::TRANSPORT_REQ
    );

    let ctx = mcs::decode_send_data_indication(output.filled()).unwrap();
    assert_eq!(
        ctx.channel_id, message_channel_id,
        "request must go out on the message channel"
    );
    let on_wire_request = decode::<MultitransportRequestPdu>(ctx.user_data).unwrap();
    assert_eq!(on_wire_request, sent_request);

    // CapabilitiesSendServer: sends Demand Active, moves on to CapabilitiesWaitConfirm
    // (no monitor-layout support was advertised).
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    assert_eq!(acceptor.state().name(), "CapabilitiesWaitConfirm");

    // The client's Initiate Multitransport Response, arriving before Confirm Active.
    // MS-RDPBCGR 2.2.15.2: S_OK MUST only be sent to a server advertising
    // SOFTSYNC_TCP_TO_UDP, which this test's offer does not include; the
    // legitimate response here is a failure code.
    let response = MultitransportResponsePdu::abort(sent_request.request_id);
    let response_bytes = encode_send_data_request(user_channel_id, message_channel_id, &encode_vec(&response).unwrap());
    let written = acceptor.step(&response_bytes, None, &mut WriteBuf::new()).unwrap();
    assert!(matches!(written, Written::Nothing));
    assert_eq!(
        acceptor.state().name(),
        "CapabilitiesWaitConfirm",
        "the response must not advance past capabilities waiting"
    );

    // Now the actual Confirm Active.
    let confirm_active = encode_send_data_request(user_channel_id, _io_channel_id, &CLIENT_DEMAND_ACTIVE_PDU_BUFFER);
    acceptor.step(&confirm_active, None, &mut WriteBuf::new()).unwrap();
    assert_eq!(acceptor.state().name(), "ConnectionFinalization");
}

/// A fixed `MultitransportSecurityRng` for deterministic assertions.
struct FixedMultitransportSecurityRng {
    cookie: [u8; 16],
    request_id: u32,
}

impl MultitransportSecurityRng for FixedMultitransportSecurityRng {
    fn fill_security_cookie(&mut self, cookie: &mut [u8; 16]) {
        *cookie = self.cookie;
    }

    fn next_request_id(&mut self) -> u32 {
        self.request_id
    }
}

/// The security cookie and request ID in the Initiate Multitransport Request
/// come from the injected `MultitransportSecurityRng`, not from a hidden
/// global RNG read inside `step()`: the acceptor is deterministic when its
/// randomness is supplied rather than sourced internally.
#[test]
fn multitransport_request_uses_the_injected_rng() {
    let mut acceptor = multitransport_acceptor(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
    acceptor.set_multitransport_security_rng(Box::new(FixedMultitransportSecurityRng {
        cookie: [0xAB; 16],
        request_id: 0x1234_5678,
    }));

    let client_blocks =
        client_gcc_with_message_channel_and_multitransport(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
    drive_to_secure_settings_exchange(&mut acceptor, client_blocks);

    // LicensingExchange (sends license) -> MultitransportBootstrapping.
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    // MultitransportBootstrapping: sends the request using the injected RNG.
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();

    let sent_request = acceptor
        .multitransport_request()
        .expect("request recorded after MultitransportBootstrapping");
    assert_eq!(sent_request.security_cookie, [0xAB; 16]);
    assert_eq!(sent_request.request_id, 0x1234_5678);
}

/// The message channel also carries Auto-Detect Response and Heartbeat PDUs
/// (MS-RDPBCGR 2.2.1.4.5, 2.2.8.1.1.2.1), not just the Initiate Multitransport
/// Response. A guard keyed only on channel and outstanding-request, without
/// verifying the payload actually decodes as a response, would misclassify
/// that other traffic and drop it silently instead of letting the caller's
/// own (pre-existing, unrelated to this fix) handling see it.
#[test]
fn non_response_traffic_on_the_message_channel_is_not_misclassified() {
    let mut acceptor = multitransport_acceptor(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));

    let client_blocks =
        client_gcc_with_message_channel_and_multitransport(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
    let (user_channel_id, _io_channel_id, message_channel_id, _) =
        drive_to_secure_settings_exchange(&mut acceptor, client_blocks);
    let message_channel_id = message_channel_id.expect("message channel negotiated");

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping: sends request
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // CapabilitiesSendServer: sends Demand Active
    assert_eq!(acceptor.state().name(), "CapabilitiesWaitConfirm");

    // Traffic on the message channel that is not a MultitransportResponsePdu
    // (its wire format is just requestId/hrResponse, so arbitrary bytes of a
    // different shape and length reliably fail that specific decode). Before
    // the fix, the old channel-only guard treated this as a response and
    // silently dropped it (`Ok(Written::Nothing)`, no change of state,
    // forever). After the fix, it falls through to the same handling any
    // other unexpected traffic already gets here (a decode error), rather
    // than being silently and permanently swallowed.
    let not_a_response = encode_send_data_request(user_channel_id, message_channel_id, &[0xAA; 3]);
    let result = acceptor.step(&not_a_response, None, &mut WriteBuf::new());
    assert!(
        result.is_err(),
        "non-response message-channel traffic must not be silently swallowed as a response"
    );
}

/// With the Basic Security Header that the acceptor's ENCRYPTION_LEVEL_NONE
/// implies, the Initiate Multitransport Response is exactly 12 bytes
/// (MS-RDPBCGR 2.2.15.2). A message-channel payload that starts with a valid
/// response but carries trailing bytes is not one, so it must not be consumed
/// as a late response: it falls through to the same handling as any other
/// unexpected traffic.
#[test]
fn multitransport_response_with_trailing_bytes_is_not_consumed() {
    let mut acceptor = multitransport_acceptor(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));

    let client_blocks =
        client_gcc_with_message_channel_and_multitransport(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
    let (user_channel_id, _io_channel_id, message_channel_id, _) =
        drive_to_secure_settings_exchange(&mut acceptor, client_blocks);
    let message_channel_id = message_channel_id.expect("message channel negotiated");

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping: sends request
    let sent_request = acceptor.multitransport_request().expect("request sent").clone();
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // CapabilitiesSendServer: sends Demand Active
    assert_eq!(acceptor.state().name(), "CapabilitiesWaitConfirm");

    let mut payload = encode_vec(&MultitransportResponsePdu::abort(sent_request.request_id)).unwrap();
    payload.push(0x00);
    let response_with_trailing_byte = encode_send_data_request(user_channel_id, message_channel_id, &payload);
    let result = acceptor.step(&response_with_trailing_byte, None, &mut WriteBuf::new());
    assert!(
        result.is_err(),
        "a response followed by trailing bytes must not be swallowed as a late response"
    );
}

/// MS-RDPBCGR 3.2.5.15.1 gives the Initiate Multitransport Response no fixed
/// position relative to the rest of the handshake: it depends on when the
/// client resolves its own bootstrapping and whether the sideband attempt
/// failed, so a conforming client can send it after Confirm Active, during
/// finalization, not only before it as `multitransport_offered_and_client_reciprocates`
/// exercises. `WaitRequestControl` is the sharpest of FinalizationSequence's
/// sub-states to hit: unlike `WaitSynchronize`/`WaitControlCooperate` (which
/// discard a decode error and advance anyway) or `WaitFontList` (which retries
/// on a decode error, tolerating one stray message), it propagates a decode
/// failure with `?`, so without tolerance the response's raw bytes (a bare
/// requestId/hrResponse pair, not a ShareControlHeader at all) fail to decode
/// and the connection is dropped outright.
#[test]
fn multitransport_response_arriving_during_finalization_is_tolerated() {
    let mut acceptor = multitransport_acceptor(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));

    let client_blocks =
        client_gcc_with_message_channel_and_multitransport(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
    let (user_channel_id, io_channel_id, message_channel_id, _) =
        drive_to_secure_settings_exchange(&mut acceptor, client_blocks);
    let message_channel_id = message_channel_id.expect("message channel negotiated");

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping: sends request
    let sent_request = acceptor.multitransport_request().expect("request sent").clone();
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // CapabilitiesSendServer: sends Demand Active

    let confirm_active = encode_send_data_request(user_channel_id, io_channel_id, &CLIENT_DEMAND_ACTIVE_PDU_BUFFER);
    acceptor.step(&confirm_active, None, &mut WriteBuf::new()).unwrap();
    assert_eq!(acceptor.state().name(), "ConnectionFinalization");

    // Drive the real Synchronize and ControlCooperate exchange first, reaching
    // WaitRequestControl.
    let synchronize = encode_send_data_request(
        user_channel_id,
        io_channel_id,
        &encode_client_share_data(ShareDataPdu::Synchronize(SynchronizePdu { target_user_id: 0 })),
    );
    acceptor.step(&synchronize, None, &mut WriteBuf::new()).unwrap();

    let cooperate = encode_send_data_request(
        user_channel_id,
        io_channel_id,
        &encode_client_share_data(ShareDataPdu::Control(ControlPdu {
            action: ControlAction::Cooperate,
            grant_id: 0,
            control_id: 0,
        })),
    );
    acceptor.step(&cooperate, None, &mut WriteBuf::new()).unwrap();

    // The late response, arriving while WaitRequestControl is active.
    // MS-RDPBCGR 2.2.15.2: S_OK MUST only be sent to a server advertising
    // SOFTSYNC_TCP_TO_UDP, which this test's offer does not include; the
    // legitimate response here is a failure code.
    let response = MultitransportResponsePdu::abort(sent_request.request_id);
    let response_bytes = encode_send_data_request(user_channel_id, message_channel_id, &encode_vec(&response).unwrap());
    let written = acceptor
        .step(&response_bytes, None, &mut WriteBuf::new())
        .expect("a late response must not drop the connection");
    assert!(matches!(written, Written::Nothing));
    assert_eq!(
        acceptor.state().name(),
        "ConnectionFinalization",
        "a late response must not be treated as RequestControl"
    );

    // The real remainder of the sequence, undisturbed by the late response above.
    let request_control = encode_send_data_request(
        user_channel_id,
        io_channel_id,
        &encode_client_share_data(ShareDataPdu::Control(ControlPdu {
            action: ControlAction::RequestControl,
            grant_id: 0,
            control_id: 0,
        })),
    );
    acceptor.step(&request_control, None, &mut WriteBuf::new()).unwrap();

    let font_list = encode_send_data_request(
        user_channel_id,
        io_channel_id,
        &encode_client_share_data(ShareDataPdu::FontList(Default::default())),
    );
    acceptor.step(&font_list, None, &mut WriteBuf::new()).unwrap();

    // The server's four confirmation sends, completing finalization.
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // SendSynchronizeConfirm
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // SendControlCooperateConfirm
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // SendGrantedControlConfirm
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // SendFontMap -> Accepted

    assert_eq!(acceptor.state().name(), "Connected");
}

/// Multitransport disabled (the default): no Server MultiTransportChannelData
/// block is advertised even though the client supports it, and no Initiate
/// Multitransport Request is ever sent.
#[test]
fn multitransport_not_offered_by_default() {
    let mut acceptor = multitransport_acceptor(None);

    let client_blocks =
        client_gcc_with_message_channel_and_multitransport(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
    let (.., server_multitransport) = drive_to_secure_settings_exchange(&mut acceptor, client_blocks);
    assert_eq!(
        server_multitransport, None,
        "server must not advertise MultiTransportChannelData when multitransport is disabled"
    );

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    let written = acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping
    assert!(matches!(written, Written::Nothing));
    assert!(acceptor.multitransport_request().is_none());
    assert_eq!(acceptor.multitransport_soft_sync_negotiated(), None);
}

/// The request decision follows what the server advertised during Basic
/// Settings Exchange, not the live offer: enabling multitransport only after
/// the GCC response went out without a MultiTransportChannelData block must
/// not produce a request the client was never told about (MS-RDPBCGR 2.2.1.4).
#[test]
fn multitransport_offer_enabled_after_basic_settings_is_not_requested() {
    let mut acceptor = multitransport_acceptor(None);

    let client_blocks =
        client_gcc_with_message_channel_and_multitransport(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
    let (.., server_multitransport) = drive_to_secure_settings_exchange(&mut acceptor, client_blocks);
    assert_eq!(server_multitransport, None);

    acceptor.set_multitransport_offer(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    let written = acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping
    assert!(matches!(written, Written::Nothing));
    assert!(acceptor.multitransport_request().is_none());
}

/// Likewise for Soft-Sync: adding `SOFT_SYNC_TCP_TO_UDP` to the offer after
/// the GCC response advertised reliable UDP without it must not make
/// `multitransport_soft_sync_negotiated()` report a negotiation that never
/// took place.
#[test]
fn soft_sync_added_after_basic_settings_is_not_negotiated() {
    let mut acceptor = multitransport_acceptor(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));

    let client_blocks = client_gcc_with_message_channel_and_multitransport(Some(
        MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR | MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP,
    ));
    let (.., server_multitransport) = drive_to_secure_settings_exchange(&mut acceptor, client_blocks);
    assert_eq!(
        server_multitransport,
        Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR)
    );

    acceptor.set_multitransport_offer(Some(
        MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR | MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP,
    ));

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping: sends request
    assert!(acceptor.multitransport_request().is_some());
    assert_eq!(acceptor.multitransport_soft_sync_negotiated(), Some(false));
}

/// The acceptor offers multitransport, but the client's GCC blocks never
/// advertised reliable UDP support: nothing is sent, and per MS-RDPBCGR
/// 2.2.1.4 the server's own GCC response must omit the block entirely
/// rather than echo the offer the client never reciprocated.
#[test]
fn multitransport_not_offered_when_client_does_not_reciprocate() {
    let mut acceptor = multitransport_acceptor(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));

    let client_blocks = client_gcc_with_message_channel_and_multitransport(None);
    let (.., server_multitransport) = drive_to_secure_settings_exchange(&mut acceptor, client_blocks);
    assert_eq!(
        server_multitransport, None,
        "server must not advertise MultiTransportChannelData when the client didn't send one"
    );

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    let written = acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping
    assert!(matches!(written, Written::Nothing));
    assert!(acceptor.multitransport_request().is_none());
}

/// Drives a fresh acceptor through the whole connection sequence, from the Connection Request to `Connected`, with
/// `monitors` sent as the client's GCC [2.2.1.3.6] Client Monitor Data (TS_UD_CS_MONITOR), or with no such block
/// when `None`.
///
/// [2.2.1.3.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/a8029f8e-01e1-4f19-af67-bcad5bdef624
fn accept(monitors: Option<Vec<Monitor>>) -> Acceptor {
    let mut client_blocks = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    client_blocks.network = None;
    client_blocks.monitor = monitors.map(|monitors| ClientMonitorData { monitors });

    let mut acceptor = multitransport_acceptor(None);

    let (user_channel_id, io_channel_id, ..) = drive_to_secure_settings_exchange(&mut acceptor, client_blocks);

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping: nothing to send
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // CapabilitiesSendServer: sends Demand Active

    let confirm_active = encode_send_data_request(user_channel_id, io_channel_id, &CLIENT_DEMAND_ACTIVE_PDU_BUFFER);
    acceptor.step(&confirm_active, None, &mut WriteBuf::new()).unwrap();

    finish_finalization(&mut acceptor, user_channel_id, io_channel_id);
    assert_eq!(acceptor.state().name(), "Connected");

    acceptor
}

/// The client's PDUs of the connection finalization, in the order it sends them.
fn client_finalization_pdus() -> [ShareDataPdu; 4] {
    [
        ShareDataPdu::Synchronize(SynchronizePdu { target_user_id: 0 }),
        ShareDataPdu::Control(ControlPdu {
            action: ControlAction::Cooperate,
            grant_id: 0,
            control_id: 0,
        }),
        ShareDataPdu::Control(ControlPdu {
            action: ControlAction::RequestControl,
            grant_id: 0,
            control_id: 0,
        }),
        ShareDataPdu::FontList(Default::default()),
    ]
}

/// Steps `sequence` through `pdus`, each sent as a client Share Data PDU on the given channels.
fn send_client_finalization_pdus(
    sequence: &mut impl Sequence,
    user_channel_id: u16,
    io_channel_id: u16,
    pdus: impl IntoIterator<Item = ShareDataPdu>,
) {
    for pdu in pdus {
        let request = encode_send_data_request(user_channel_id, io_channel_id, &encode_client_share_data(pdu));
        sequence.step(&request, None, &mut WriteBuf::new()).unwrap();
    }
}

/// Steps `sequence` through the whole connection finalization: the client's four PDUs, then the server's four
/// confirmation sends.
fn finish_finalization(sequence: &mut impl Sequence, user_channel_id: u16, io_channel_id: u16) {
    send_client_finalization_pdus(sequence, user_channel_id, io_channel_id, client_finalization_pdus());
    for _ in 0..4 {
        sequence.step(&[], None, &mut WriteBuf::new()).unwrap();
    }
}

/// Returns the monitors the acceptor hands back in its result after [`accept`] with `monitors`.
fn client_monitors_after_accepting(monitors: Option<Vec<Monitor>>) -> Vec<Monitor> {
    accept(monitors)
        .get_result()
        .expect("a connected acceptor has a result")
        .client_monitors
}

/// The client's monitors reach `AcceptorResult::client_monitors` as sent,
/// order included: the primary monitor comes second here.
#[test]
fn client_monitor_data_is_kept_in_the_order_sent() {
    let monitors = vec![
        Monitor {
            left: 1280,
            top: 0,
            right: 2559,
            bottom: 799,
            flags: MonitorFlags::empty(),
        },
        Monitor {
            left: 0,
            top: 0,
            right: 1279,
            bottom: 799,
            flags: MonitorFlags::PRIMARY,
        },
    ];

    assert_eq!(client_monitors_after_accepting(Some(monitors.clone())), monitors);
}

#[test]
fn a_client_without_monitor_data_announces_no_monitors() {
    assert!(client_monitors_after_accepting(None).is_empty());
}

fn complete_activation(acceptor: &mut Acceptor, user_channel_id: u16, io_channel_id: u16) {
    let confirm = encode_send_data_request(user_channel_id, io_channel_id, &CLIENT_DEMAND_ACTIVE_PDU_BUFFER);
    acceptor.step(&confirm, None, &mut WriteBuf::new()).unwrap();
    complete_finalization(acceptor, user_channel_id, io_channel_id);
}

fn complete_finalization(acceptor: &mut Acceptor, user_channel_id: u16, io_channel_id: u16) {
    for pdu in [
        ShareDataPdu::Synchronize(SynchronizePdu { target_user_id: 0 }),
        ShareDataPdu::Control(ControlPdu {
            action: ControlAction::Cooperate,
            grant_id: 0,
            control_id: 0,
        }),
        ShareDataPdu::Control(ControlPdu {
            action: ControlAction::RequestControl,
            grant_id: 0,
            control_id: 0,
        }),
        ShareDataPdu::FontList(Default::default()),
    ] {
        let frame = encode_send_data_request(user_channel_id, io_channel_id, &encode_client_share_data(pdu));
        acceptor.step(&frame, None, &mut WriteBuf::new()).unwrap();
    }
    for _ in 0..4 {
        acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    }
}

fn reactivating_dvc_acceptor() -> (Acceptor, u16, u16, Vec<u8>) {
    let desktop = DesktopSize {
        width: 1280,
        height: 720,
    };
    let mut acceptor = Acceptor::new(SecurityProtocol::SSL, desktop, Vec::new(), None);
    acceptor.attach_static_channel(DrdynvcServer::new().with_dynamic_channel(EchoServer::new()));
    let mut blocks = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    blocks.network = Some(ironrdp_pdu::gcc::ClientNetworkData {
        channels: vec![ironrdp_svc::make_channel_definition(&StaticVirtualChannel::new(
            DrdynvcServer::new(),
        ))],
    });
    let (user, io, ..) = drive_to_secure_settings_exchange(&mut acceptor, blocks);
    for _ in 0..3 {
        acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    }
    complete_activation(&mut acceptor, user, io);
    let mut result = acceptor.get_result().unwrap();
    let channel_id = result
        .static_channels
        .get_channel_id_by_type::<DrdynvcServer>()
        .unwrap();
    let dvc = result
        .static_channels
        .get_by_type_mut::<DrdynvcServer>()
        .unwrap()
        .channel_processor_downcast_mut::<DrdynvcServer>()
        .unwrap();
    // oracle: spec MS-RDPEDYC §2.2.1.2: version 2 Capability Response.
    dvc.process(&[0x50, 0x00, 0x02, 0x00]).unwrap();
    let echo_id = dvc.get_channel_id_by_type::<EchoServer>().unwrap();
    let reply = client_encode_svc_messages(
        vec![SvcMessage::from(DrdynvcClientPdu::Create(CreateResponsePdu::new(
            echo_id,
            CreationStatus::OK,
        )))],
        channel_id,
        user,
    )
    .unwrap();
    let mut acceptor = Acceptor::new_deactivation_reactivation(acceptor, result.static_channels, desktop).unwrap();
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    (acceptor, user, io, reply)
}

#[test]
fn dvc_create_reply_before_reactivation_confirm_is_preserved() {
    // oracle: spec MS-RDPBCGR §1.3.3: each virtual channel is an independent, lossless stream.
    let (mut acceptor, user, io, reply) = reactivating_dvc_acceptor();
    acceptor.step(&reply, None, &mut WriteBuf::new()).unwrap();
    complete_activation(&mut acceptor, user, io);
    let result = acceptor.get_result().unwrap();
    assert_eq!(result.input_events, vec![reply]);
}

#[test]
fn dvc_create_reply_does_not_advance_reactivation_finalization() {
    // oracle: spec MS-RDPBCGR §1.3.3: virtual-channel data is independent of I/O-channel finalization.
    let (mut acceptor, user, io, reply) = reactivating_dvc_acceptor();
    let confirm = encode_send_data_request(user, io, &CLIENT_DEMAND_ACTIVE_PDU_BUFFER);
    acceptor.step(&confirm, None, &mut WriteBuf::new()).unwrap();
    acceptor.step(&reply, None, &mut WriteBuf::new()).unwrap();
    complete_finalization(&mut acceptor, user, io);
    let result = acceptor.get_result().unwrap();
    assert_eq!(result.input_events, vec![reply]);
}

#[test]
fn reactivation_backlog_rejects_frames_beyond_sixteen_mib() {
    // oracle: mutant accepting virtual-channel frames past the 16 MiB reactivation backlog cap.
    let (mut acceptor, user, _, reply) = reactivating_dvc_acceptor();
    let channel = decode::<X224<mcs::SendDataRequest<'_>>>(&reply).unwrap().0.channel_id;
    let mut payload = vec![0; 1009];
    payload[..4].copy_from_slice(&1001u32.to_le_bytes());
    payload[4..8].copy_from_slice(&3u32.to_le_bytes());
    let frame = encode_send_data_request(user, channel, &payload);
    // oracle: spec MS-RDPBCGR §2.2.6.1: 7-byte TPKT/X.224 + 8-byte MCS + 1009-byte user data.
    assert_eq!(frame.len(), 1024);
    for _ in 0..16 * 1024 {
        acceptor.step(&frame, None, &mut WriteBuf::new()).unwrap();
    }
    let error = acceptor.step(&frame, None, &mut WriteBuf::new()).unwrap_err();
    assert!(error.to_string().contains("backlog"));
}

#[derive(Default)]
struct HandshakeStream {
    incoming: VecDeque<Vec<u8>>,
    outgoing: Rc<RefCell<Vec<Vec<u8>>>>,
    credssp_client: Option<CredsspSequence>,
    fail_writes: bool,
    spnego_credssp_client: Option<Box<CredSspClient>>,
}

impl StreamWrapper for HandshakeStream {
    type InnerStream = Self;

    fn from_inner(stream: Self) -> Self {
        stream
    }

    fn into_inner(self) -> Self {
        self
    }

    fn get_inner(&self) -> &Self {
        self
    }

    fn get_inner_mut(&mut self) -> &mut Self {
        self
    }
}

impl FramedRead for HandshakeStream {
    type ReadFut<'read> = Ready<io::Result<usize>>;

    fn read<'a>(&'a mut self, buf: &'a mut BytesMut) -> Self::ReadFut<'a> {
        let chunk = self.incoming.pop_front().unwrap_or_default();
        buf.extend_from_slice(&chunk);
        ready(Ok(chunk.len()))
    }
}

impl FramedWrite for HandshakeStream {
    type WriteAllFut<'write> = Ready<io::Result<()>>;

    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a> {
        if self.fail_writes {
            return ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)));
        }
        self.outgoing.borrow_mut().push(buf.to_vec());
        if let Some(client) = self.credssp_client.as_mut()
            && buf.len() != 4
        {
            let request = client.decode_server_message(buf).unwrap().unwrap();
            // A request with an error code ends the exchange; the client does not answer it.
            if request.error_code.is_none() {
                self.incoming.push_back(client_credssp_message(client, request));
            }
        }
        if let Some(client) = self.spnego_credssp_client.as_mut()
            && buf.len() != 4
        {
            let request = TsRequest::from_buffer(buf).unwrap();
            if request.error_code.is_none() {
                self.incoming.push_back(spnego_credssp_message(client, request));
            }
        }
        ready(Ok(()))
    }
}

fn client_credssp_message(client: &mut CredsspSequence, request: TsRequest) -> Vec<u8> {
    let result = {
        let mut generator = client.process_ts_request(request);
        let GeneratorState::Completed(result) = generator.start() else {
            panic!("NTLM should not make network requests");
        };
        result.unwrap()
    };
    let mut buf = WriteBuf::new();
    client.handle_process_result(result, &mut buf).unwrap();
    buf.filled().to_vec()
}

/// Builds a CredSSP client that wraps NTLM in SPNEGO, as mstsc does.
/// The connector's client sends bare NTLM instead.
fn spnego_credssp_client(username: &str, password: &str, public_key: &[u8]) -> CredSspClient {
    CredSspClient::new(
        public_key.to_vec(),
        AuthIdentity {
            username: Username::new(username, None).unwrap(),
            password: password.to_owned().into(),
        }
        .into(),
        CredSspMode::WithCredentials,
        ClientMode::Negotiate(NegotiateConfig {
            protocol_config: Box::new(NtlmConfig::default()),
            package_list: Some("!kerberos,!pku2u".to_owned()),
            client_computer_name: "client.example.com".to_owned(),
        }),
        "TERMSRV/server.example.com".to_owned(),
    )
    .unwrap()
}

fn spnego_credssp_message(client: &mut CredSspClient, request: TsRequest) -> Vec<u8> {
    let state = {
        let mut generator = client.process(request);
        let GeneratorState::Completed(state) = generator.start() else {
            panic!("NTLM should not make network requests");
        };
        state.unwrap()
    };
    let (ClientState::ReplyNeeded(reply) | ClientState::FinalMessage(reply)) = state;
    let mut message = Vec::new();
    reply.encode_ts_request(&mut message).unwrap();
    message
}

struct NoNetwork;

impl NetworkClient for NoNetwork {
    async fn send(&mut self, _: &NetworkRequest) -> ConnectorResult<Vec<u8>> {
        Err(ConnectorError::general("unexpected network request during NTLM"))
    }
}

#[derive(Default)]
struct SetupHandler {
    received: Vec<ReceivedCredentials>,
    client_info: Option<ClientInfoFlags>,
    client_info_at_capabilities: Option<ClientInfoFlags>,
    reject_credentials: bool,
    reject_capabilities: bool,
    capability_calls: usize,
}

#[async_trait::async_trait(?Send)]
impl ConnectionSetupHandler for SetupHandler {
    async fn handle_client_info(&mut self, flags: ClientInfoFlags) -> ConnectorResult<()> {
        self.client_info = Some(flags);
        Ok(())
    }

    async fn handle_credentials(&mut self, credentials: Option<ReceivedCredentials>) -> ConnectorResult<()> {
        self.received
            .push(credentials.expect("handshake should supply credentials"));
        if self.reject_credentials {
            Err(ConnectorError::general("test credential rejection"))
        } else {
            Ok(())
        }
    }

    async fn prepare_capability_exchange(&mut self, _: DesktopSize) -> ConnectorResult<()> {
        self.capability_calls += 1;
        self.client_info_at_capabilities = self.client_info;
        if self.reject_capabilities {
            Err(ConnectorError::general("test capability rejection"))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn client_info_reaches_setup_before_capability_exchange() {
    // oracle: spec MS-RDPERP §3.3.5.1.3-.4: Client Info selects RemoteApp before Demand Active is constructed.
    let mut acceptor = negotiated_acceptor(SecurityProtocol::SSL);
    let mut handler = SetupHandler {
        reject_capabilities: true,
        ..SetupHandler::default()
    };
    let mut incoming = post_credssp_messages(None);
    let mut info = ClientInfoPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::INFO_PKT,
        },
        client_info: CLIENT_INFO_UNICODE.clone(),
    };
    info.client_info.flags |= ClientInfoFlags::RAIL | ClientInfoFlags::HIDEF_RAIL_SUPPORTED;
    *incoming.back_mut().unwrap() = encode_send_data_request(1002, 1003, &encode_vec(&info).unwrap());
    let framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming,
        ..HandshakeStream::default()
    });

    let result = accept_finalize_with(framed, &mut acceptor, &mut handler).await;

    assert!(result.err().unwrap().to_string().contains("test capability rejection"));
    assert_eq!(handler.client_info_at_capabilities, Some(info.client_info.flags));
}

fn negotiated_acceptor(protocol: SecurityProtocol) -> Acceptor {
    negotiated_acceptor_expecting(
        protocol,
        protocol
            .intersects(SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX)
            .then(|| Credentials {
                username: "alice".to_owned(),
                password: "secret".to_owned(),
                domain: None,
            }),
    )
}

/// Returns an acceptor for `protocol` that expects `creds`, past the security upgrade.
fn negotiated_acceptor_expecting(protocol: SecurityProtocol, creds: Option<Credentials>) -> Acceptor {
    let mut acceptor = Acceptor::new(
        protocol,
        DesktopSize {
            width: 1024,
            height: 768,
        },
        Vec::new(),
        creds,
    );
    acceptor
        .step(&encode_connection_request(protocol), None, &mut WriteBuf::new())
        .unwrap();
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    acceptor.mark_security_upgrade_as_done();
    acceptor
}

fn post_credssp_messages(error_info: Option<ClientEarlyCapabilityFlags>) -> VecDeque<Vec<u8>> {
    let mut gcc = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    gcc.network = None;
    gcc.core.optional_data = ClientCoreOptionalData {
        post_beta2_color_depth: gcc.core.optional_data.post_beta2_color_depth,
        client_product_id: gcc.core.optional_data.client_product_id,
        serial_number: gcc.core.optional_data.serial_number,
        high_color_depth: gcc.core.optional_data.high_color_depth,
        supported_color_depths: gcc.core.optional_data.supported_color_depths,
        early_capability_flags: error_info,
        ..ClientCoreOptionalData::default()
    };
    let mut initial = WriteBuf::new();
    encode_x224_packet(&ConnectInitial::with_gcc_blocks(gcc).unwrap(), &mut initial).unwrap();

    let info = ClientInfoPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::INFO_PKT,
        },
        client_info: CLIENT_INFO_UNICODE.clone(),
    };
    [
        initial.filled().to_vec(),
        encode_vec(&X224(mcs::ErectDomainPdu {
            sub_height: 0,
            sub_interval: 0,
        }))
        .unwrap(),
        encode_vec(&X224(mcs::AttachUserRequest)).unwrap(),
        encode_vec(&X224(mcs::ChannelJoinRequest {
            initiator_id: 1002,
            channel_id: 1002,
        }))
        .unwrap(),
        encode_vec(&X224(mcs::ChannelJoinRequest {
            initiator_id: 1002,
            channel_id: 1003,
        }))
        .unwrap(),
        encode_vec(&X224(mcs::SendDataRequest {
            initiator_id: 1002,
            channel_id: 1003,
            user_data: encode_vec(&info).unwrap().into(),
        }))
        .unwrap(),
    ]
    .into()
}

fn assert_access_denied(bytes: &[u8]) {
    let data = decode::<X224<mcs::SendDataIndication<'_>>>(bytes).unwrap().0;
    assert_eq!(data.initiator_id, 1002);
    assert_eq!(data.channel_id, 1003);
    assert_eq!(data.user_data.len(), 22);
    let header = decode::<ShareControlHeader>(data.user_data.as_ref()).unwrap();
    assert_eq!(header.pdu_source, 0);
    let ShareControlPdu::Data(data) = header.share_control_pdu else {
        panic!("expected a Share Data PDU");
    };
    assert!(data.compression_flags.is_empty());
    assert_eq!(
        data.share_data_pdu,
        ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(ErrorInfo::ProtocolIndependentCode(
            ProtocolIndependentCode::ServerInsufficientPrivileges,
        )))
    );
}

#[rstest::rstest]
#[case(None)]
#[case(Some(ClientEarlyCapabilityFlags::empty()))]
#[case(Some(ClientEarlyCapabilityFlags::SUPPORT_ERR_INFO_PDU))]
#[tokio::test]
async fn tls_hook_rejection_respects_error_info_support(
    #[case] error_info: Option<ClientEarlyCapabilityFlags>,
    #[values(false, true)] reject_credentials: bool,
) {
    let mut acceptor = negotiated_acceptor(SecurityProtocol::SSL);
    let mut handler = SetupHandler {
        reject_credentials,
        reject_capabilities: !reject_credentials,
        ..SetupHandler::default()
    };
    let framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: post_credssp_messages(error_info),
        ..HandshakeStream::default()
    });
    let outgoing = Rc::clone(&framed.get_inner().0.outgoing);
    let result = accept_finalize_with(framed, &mut acceptor, &mut handler).await;
    let error = result.err().expect("hook rejection should fail the handshake");
    assert!(error.to_string().contains(if reject_credentials {
        "test credential rejection"
    } else {
        "test capability rejection"
    }));
    assert_eq!(handler.received.len(), 1);
    assert_eq!(
        handler.received[0].origin,
        ironrdp_acceptor::CredentialOrigin::ClientInfo
    );
    assert_eq!(handler.capability_calls, usize::from(!reject_credentials));

    let outgoing = outgoing.borrow();
    let supports_error_info =
        error_info.is_some_and(|flags| flags.contains(ClientEarlyCapabilityFlags::SUPPORT_ERR_INFO_PDU));
    assert_eq!(
        outgoing.len(),
        4 + usize::from(!reject_credentials) + usize::from(supports_error_info)
    );
    assert_eq!(
        decode::<X224<mcs::ChannelJoinConfirm>>(&outgoing[3])
            .unwrap()
            .0
            .channel_id,
        1003
    );
    let mut denied = WriteBuf::new();
    let written = acceptor.encode_access_denied(&mut denied).unwrap();
    if supports_error_info {
        assert_eq!(written, denied.filled_len());
        assert_access_denied(denied.filled());
        assert_eq!(outgoing.last().unwrap(), denied.filled());
    } else {
        assert_eq!(written, 0);
        assert!(denied.filled().is_empty());
    }
}

// oracle: spec MS-RDPBCGR §2.2.1.2.2 (the server informs the client of a security negotiation failure with an RDP
// Negotiation Failure)
// oracle: mutant driver dropping what a failing step wrote
#[tokio::test]
async fn accept_begin_sends_the_negotiation_failure() {
    let mut acceptor = Acceptor::new(
        SecurityProtocol::SSL,
        DesktopSize {
            width: 1024,
            height: 768,
        },
        Vec::new(),
        None,
    );
    let framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: [encode_connection_request(SecurityProtocol::HYBRID)].into(),
        ..HandshakeStream::default()
    });
    let outgoing = Rc::clone(&framed.get_inner().0.outgoing);

    let result = accept_begin(framed, &mut acceptor).await;
    assert!(
        result.is_err(),
        "a security protocol mismatch should fail the handshake"
    );

    let outgoing = outgoing.borrow();
    let [failure] = outgoing.as_slice() else {
        panic!("expected only the RDP Negotiation Failure, got {} PDUs", outgoing.len());
    };
    assert_eq!(
        decode::<X224<nego::ConnectionConfirm>>(failure).unwrap().0,
        nego::ConnectionConfirm::Failure {
            code: nego::FailureCode::SSL_REQUIRED_BY_SERVER,
        }
    );
}

// oracle: spec MS-RDPBCGR §2.2.5.1.1 (the fields of the Set Error Info PDU that `assert_access_denied` checks)
// oracle: mutant driver dropping what a failing step wrote
#[tokio::test]
async fn accept_finalize_sends_the_error_info_for_other_credentials() {
    let mut acceptor = negotiated_acceptor_expecting(
        SecurityProtocol::SSL,
        Some(Credentials {
            username: "mallory".to_owned(),
            password: "secret".to_owned(),
            domain: None,
        }),
    );
    let framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: post_credssp_messages(Some(ClientEarlyCapabilityFlags::SUPPORT_ERR_INFO_PDU)),
        ..HandshakeStream::default()
    });
    let outgoing = Rc::clone(&framed.get_inner().0.outgoing);

    let error = accept_finalize(framed, &mut acceptor)
        .await
        .err()
        .expect("other credentials should fail the handshake");
    assert!(
        matches!(error.kind(), ConnectorErrorKind::General),
        "expected the credentials rejection, got {error}"
    );

    // The Connect Response, the Attach User Confirm and two Channel Join Confirms come first.
    let outgoing = outgoing.borrow();
    assert_eq!(outgoing.len(), 5);
    assert_access_denied(&outgoing[4]);
}

// oracle: spec MS-RDPBCGR §2.2.5.1 (the Set Error Info PDU is only sent to clients that set
// RNS_UD_CS_SUPPORT_ERRINFO_PDU)
// oracle: mutant Client Info step writing the Set Error Info PDU without checking the client supports it
#[rstest::rstest]
#[case::no_early_capability_flags(None)]
#[case::no_error_info_flag(Some(ClientEarlyCapabilityFlags::empty()))]
#[tokio::test]
async fn accept_finalize_sends_no_error_info_to_a_client_without_support(
    #[case] error_info: Option<ClientEarlyCapabilityFlags>,
) {
    let mut acceptor = negotiated_acceptor_expecting(
        SecurityProtocol::SSL,
        Some(Credentials {
            username: "mallory".to_owned(),
            password: "secret".to_owned(),
            domain: None,
        }),
    );
    let framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: post_credssp_messages(error_info),
        ..HandshakeStream::default()
    });
    let outgoing = Rc::clone(&framed.get_inner().0.outgoing);

    let error = accept_finalize(framed, &mut acceptor)
        .await
        .err()
        .expect("other credentials should fail the handshake");
    assert!(
        matches!(error.kind(), ConnectorErrorKind::General),
        "expected the credentials rejection, got {error}"
    );

    // Only the Connect Response, the Attach User Confirm and two Channel Join Confirms.
    assert_eq!(outgoing.borrow().len(), 4);
}

fn step_client_info_credentials(
    expected_username: &str,
    expected_domain: Option<&str>,
    actual_username: &str,
    actual_domain: Option<&str>,
) -> ConnectorResult<Acceptor> {
    let mut acceptor = negotiated_acceptor_expecting(
        SecurityProtocol::SSL,
        Some(Credentials {
            username: expected_username.to_owned(),
            password: "secret".to_owned(),
            domain: expected_domain.map(str::to_owned),
        }),
    );
    let mut incoming = post_credssp_messages(None);
    let mut info = ClientInfoPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::INFO_PKT,
        },
        client_info: CLIENT_INFO_UNICODE.clone(),
    };
    info.client_info.credentials = Credentials {
        username: actual_username.to_owned(),
        password: "secret".to_owned(),
        domain: actual_domain.map(str::to_owned),
    };
    *incoming.back_mut().unwrap() = encode_send_data_request(1002, 1003, &encode_vec(&info).unwrap());

    while let Some(message) = incoming.pop_front() {
        while acceptor.next_pdu_hint().is_none() {
            acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
        }
        acceptor.step(&message, None, &mut WriteBuf::new())?;
    }
    Ok(acceptor)
}

// oracle: mutant rejecting a Client Info UPN or Down-Level Logon Name that matches the configured domain.
#[rstest::rstest]
#[case::upn("alice@corp")]
#[case::down_level("CORP\\alice")]
fn client_info_matches_qualified_username_against_expected_domain(#[case] username: &str) {
    let acceptor = step_client_info_credentials("alice", Some("CORP"), username, None).unwrap();
    assert_eq!(acceptor.received_credentials().unwrap().credentials.username, username);
}

// oracle: mutant accepting a Client Info username whose domain is missing or does not match the configured domain.
#[rstest::rstest]
#[case::different_upn_realm("alice@evil.example")]
#[case::different_down_level_domain("OTHER\\alice")]
#[case::unqualified("alice")]
fn client_info_rejects_username_with_mismatched_or_missing_domain(#[case] username: &str) {
    assert!(step_client_info_credentials("alice", Some("CORP"), username, None).is_err());
}

// oracle: mutant driver returning the error of sending the failing step's PDU instead of the step error
#[tokio::test]
async fn accept_begin_reports_the_step_error_when_sending_its_pdu_fails() {
    let mut acceptor = Acceptor::new(
        SecurityProtocol::SSL,
        DesktopSize {
            width: 1024,
            height: 768,
        },
        Vec::new(),
        None,
    );
    let framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: [encode_connection_request(SecurityProtocol::HYBRID)].into(),
        fail_writes: true,
        ..HandshakeStream::default()
    });

    let error = accept_begin(framed, &mut acceptor)
        .await
        .err()
        .expect("a security protocol mismatch should fail the handshake");
    assert!(
        matches!(error.kind(), ConnectorErrorKind::Reason(_)),
        "expected the security protocol mismatch, got {error}"
    );
}

#[rstest::rstest]
#[case(SecurityProtocol::HYBRID, None)]
#[case(SecurityProtocol::HYBRID, Some(ClientEarlyCapabilityFlags::SUPPORT_ERR_INFO_PDU))]
#[case(SecurityProtocol::HYBRID_EX, None)]
#[tokio::test]
async fn credssp_authorization_uses_selected_protocol(
    #[case] protocol: SecurityProtocol,
    #[case] error_info: Option<ClientEarlyCapabilityFlags>,
    #[values("alice@corp.example.com", "CORP\\alice", "alice")] username: &str,
    #[values(false, true)] reject_credentials: bool,
) {
    let mut acceptor = negotiated_acceptor(protocol);
    let public_key = vec![0x42; 32];
    let (mut client, request) = CredsspSequence::init(
        ironrdp_connector::Credentials::UsernamePassword {
            username: username.to_owned(),
            password: "secret".to_owned(),
        },
        None,
        protocol,
        "server.example.com".to_owned().into(),
        public_key.clone(),
        None,
    )
    .unwrap();
    let first_message = client_credssp_message(&mut client, request);
    let mut framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: [first_message].into(),
        credssp_client: Some(client),
        ..HandshakeStream::default()
    });
    let outgoing = Rc::clone(&framed.get_inner().0.outgoing);
    let mut handler = SetupHandler {
        reject_credentials,
        reject_capabilities: !reject_credentials,
        ..SetupHandler::default()
    };
    let result = accept_credssp_with(
        &mut framed,
        &mut acceptor,
        &mut NoNetwork,
        "client.example.com".to_owned().into(),
        public_key,
        None,
        &mut handler,
    )
    .await;

    if protocol == SecurityProtocol::HYBRID_EX && reject_credentials {
        assert!(result.unwrap_err().to_string().contains("test credential rejection"));
        assert_eq!(outgoing.borrow().last().unwrap(), &5u32.to_le_bytes());
    } else {
        result.unwrap();
        if protocol == SecurityProtocol::HYBRID {
            assert!(handler.received.is_empty(), "plain HYBRID must defer authorization");
        } else {
            assert_eq!(handler.received.len(), 1);
            assert_eq!(outgoing.borrow().last().unwrap(), &0u32.to_le_bytes());
        }
        let credssp_write_count = outgoing.borrow().len();
        let (stream, _) = framed.get_inner_mut();
        stream.credssp_client = None;
        stream.incoming = post_credssp_messages(error_info);
        let error = accept_finalize_with(framed, &mut acceptor, &mut handler)
            .await
            .err()
            .expect("hook rejection should fail the handshake");
        assert!(error.to_string().contains(if reject_credentials {
            "test credential rejection"
        } else {
            "test capability rejection"
        }));
        let outgoing = outgoing.borrow();
        let handshake_writes = &outgoing[credssp_write_count..];
        let supports_error_info =
            error_info.is_some_and(|flags| flags.contains(ClientEarlyCapabilityFlags::SUPPORT_ERR_INFO_PDU));
        assert_eq!(
            handshake_writes.len(),
            4 + usize::from(!reject_credentials) + usize::from(supports_error_info)
        );
        assert_eq!(
            decode::<X224<mcs::ChannelJoinConfirm>>(&handshake_writes[3])
                .unwrap()
                .0
                .channel_id,
            1003
        );
        if supports_error_info {
            assert_access_denied(handshake_writes.last().unwrap());
        }
    }

    assert_eq!(handler.received.len(), 1);
    assert_eq!(
        handler.received[0].origin,
        ironrdp_acceptor::CredentialOrigin::CredSspDelegated
    );
    let (expected_username, expected_domain) = match username {
        "CORP\\alice" => ("alice", Some("CORP")),
        _ => (username, None),
    };
    assert_eq!(handler.received[0].credentials.username, expected_username);
    assert_eq!(handler.received[0].credentials.domain.as_deref(), expected_domain);
    assert_eq!(handler.received[0].credentials.password, "secret");
    assert_eq!(handler.capability_calls, usize::from(!reject_credentials));
}

/// A failed NLA logon is reported with STATUS_LOGON_FAILURE ([MS-CSSP] 2.2.1, [MS-ERREF] 2.3.1),
/// so that the client can ask for new credentials.
#[rstest::rstest]
#[case::wrong_password("alice", "wrong")]
#[case::unknown_user("bob", "secret")]
#[tokio::test]
async fn credssp_failed_logon_is_reported_as_logon_failure(
    #[case] username: &str,
    #[case] password: &str,
    #[values(SecurityProtocol::HYBRID, SecurityProtocol::HYBRID_EX)] protocol: SecurityProtocol,
) {
    let mut acceptor = negotiated_acceptor(protocol);
    let public_key = vec![0x42; 32];
    let (mut client, request) = CredsspSequence::init(
        ironrdp_connector::Credentials::UsernamePassword {
            username: username.to_owned(),
            password: password.to_owned(),
        },
        None,
        protocol,
        "server.example.com".to_owned().into(),
        public_key.clone(),
        None,
    )
    .unwrap();
    let first_message = client_credssp_message(&mut client, request);
    let mut framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: [first_message].into(),
        credssp_client: Some(client),
        ..HandshakeStream::default()
    });
    let outgoing = Rc::clone(&framed.get_inner().0.outgoing);
    let mut handler = SetupHandler::default();
    let result = accept_credssp_with(
        &mut framed,
        &mut acceptor,
        &mut NoNetwork,
        "client.example.com".to_owned().into(),
        public_key,
        None,
        &mut handler,
    )
    .await;

    assert!(result.is_err(), "the logon should fail");
    assert!(handler.received.is_empty());

    let outgoing = outgoing.borrow();
    let mut writes = outgoing.iter().rev();
    if protocol == SecurityProtocol::HYBRID_EX {
        assert_eq!(writes.next().unwrap(), &5u32.to_le_bytes());
    }
    let ts_request = TsRequest::from_buffer(writes.next().unwrap()).unwrap();
    assert_eq!(ts_request.error_code, Some(NStatusCode::LOGON_FAILURE));
}

/// The server Demand Active carries `RDP` as its source descriptor, as the FreeRDP
/// server does; some clients (Jump Desktop) refuse an empty one.
#[test]
fn demand_active_uses_rdp_source_descriptor() {
    let mut acceptor = multitransport_acceptor(None);
    drive_to_secure_settings_exchange(&mut acceptor, client_gcc_with_message_channel_and_multitransport(None));

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping
    let mut output = WriteBuf::new();
    acceptor.step(&[], None, &mut output).unwrap(); // CapabilitiesSendServer: sends Demand Active

    let indication = mcs::decode_send_data_indication(output.filled()).unwrap();
    let ShareControlPdu::ServerDemandActive(demand_active) = decode_share_control(indication).unwrap().pdu else {
        panic!("expected a Server Demand Active PDU");
    };
    assert_eq!(demand_active.pdu.source_descriptor, "RDP");
}

// oracle: regression Devolutions/IronRDP#1325 (stepping an acceptor after a failed step panicked instead of returning
// an error, https://github.com/Devolutions/IronRDP/issues/1325#issuecomment-6052032954)
#[test]
fn stepping_a_failed_acceptor_returns_an_error() {
    let mut acceptor = multitransport_acceptor(None);
    acceptor
        .step(b"not a connection request", None, &mut WriteBuf::new())
        .unwrap_err();

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap_err();
}

// oracle: regression Devolutions/IronRDP#1325 (stepping an accepted acceptor panicked instead of returning an error,
// and after its result was taken, https://github.com/Devolutions/IronRDP/issues/1325#issuecomment-6052032954)
#[test]
fn stepping_an_accepted_acceptor_returns_an_error() {
    let mut acceptor = accept(None);

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap_err();
    // oracle: mutant the Accepted arm of `Acceptor::step` not putting the state back, losing the accepted result
    assert!(acceptor.get_result().is_some());

    // Taking the result leaves the acceptor consumed, and a step then errors as after a failed step.
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap_err();
}

// oracle: regression Devolutions/IronRDP#1325 (the fix of the reported acceptor panic was requested to cover
// channel_connection.rs, whose sequence reached the same unreachable!() after a failed step,
// https://github.com/Devolutions/IronRDP/issues/1325#issuecomment-6052032954)
#[test]
fn stepping_a_failed_channel_connection_returns_an_error() {
    let mut sequence = ChannelConnectionSequence::skip_channel_join(USER_CHANNEL_ID);
    sequence
        .step(b"not an erect domain request", None, &mut WriteBuf::new())
        .unwrap_err();

    sequence.step(&[], None, &mut WriteBuf::new()).unwrap_err();
}

// oracle: regression Devolutions/IronRDP#1325 (the fix of the reported acceptor panic was requested to cover
// channel_connection.rs, whose sequence reached the same unreachable!() once every channel was joined,
// https://github.com/Devolutions/IronRDP/issues/1325#issuecomment-6052032954)
#[test]
fn stepping_a_finished_channel_connection_returns_an_error() {
    // A channel connection that skips the channel joins ends with the Attach User Confirm.
    let mut sequence = ChannelConnectionSequence::skip_channel_join(USER_CHANNEL_ID);
    let erect_domain = encode_vec(&X224(mcs::ErectDomainPdu {
        sub_height: 0,
        sub_interval: 0,
    }))
    .unwrap();
    sequence.step(&erect_domain, None, &mut WriteBuf::new()).unwrap();
    let attach_user = encode_vec(&X224(mcs::AttachUserRequest)).unwrap();
    sequence.step(&attach_user, None, &mut WriteBuf::new()).unwrap();
    sequence.step(&[], None, &mut WriteBuf::new()).unwrap(); // Attach User Confirm
    assert!(sequence.is_done());

    sequence.step(&[], None, &mut WriteBuf::new()).unwrap_err();
    // oracle: mutant the AllJoined arm of `ChannelConnectionSequence::step` not putting the state back
    assert!(sequence.is_done());
}

// oracle: regression Devolutions/IronRDP#1325 (the fix of the reported acceptor panic was requested to cover
// finalization.rs, whose sequence reached the same unreachable!() once it finished,
// https://github.com/Devolutions/IronRDP/issues/1325#issuecomment-6052032954)
#[test]
fn stepping_a_finished_finalization_returns_an_error() {
    // The client's four PDUs and the server's four answers finish the finalization.
    let mut sequence = FinalizationSequence::new(USER_CHANNEL_ID, IO_CHANNEL_ID);
    finish_finalization(&mut sequence, USER_CHANNEL_ID, IO_CHANNEL_ID);
    assert!(sequence.is_done());

    sequence.step(&[], None, &mut WriteBuf::new()).unwrap_err();
    let vc_frame = encode_send_data_request(USER_CHANNEL_ID, IO_CHANNEL_ID + 1, b"vc");
    // oracle: mutant buffering non-I/O-channel frames after finalization has already finished.
    sequence.step(&vc_frame, None, &mut WriteBuf::new()).unwrap_err();
    // oracle: mutant the Finished arm of `FinalizationSequence::step` not putting the state back
    assert!(sequence.is_done());
}

// oracle: regression Devolutions/IronRDP#1325 (the fix of the reported acceptor panic was requested to cover
// finalization.rs, whose sequence reached the same unreachable!() after a failed step,
// https://github.com/Devolutions/IronRDP/issues/1325#issuecomment-6052032954)
#[test]
fn stepping_a_failed_finalization_returns_an_error() {
    // The Synchronize and Control Cooperate states ignore what they read, so the Request Control state is the first
    // that a bad PDU fails.
    let mut sequence = FinalizationSequence::new(USER_CHANNEL_ID, IO_CHANNEL_ID);
    send_client_finalization_pdus(
        &mut sequence,
        USER_CHANNEL_ID,
        IO_CHANNEL_ID,
        client_finalization_pdus().into_iter().take(2),
    );
    sequence
        .step(b"not a request control PDU", None, &mut WriteBuf::new())
        .unwrap_err();

    sequence.step(&[], None, &mut WriteBuf::new()).unwrap_err();
    let vc_frame = encode_send_data_request(USER_CHANNEL_ID, IO_CHANNEL_ID + 1, b"vc");
    // oracle: mutant buffering non-I/O-channel frames after finalization has already failed.
    sequence.step(&vc_frame, None, &mut WriteBuf::new()).unwrap_err();
}

// oracle: spec MS-RDPBCGR §2.2.12.1 (the pduSource field of the Monitor Layout PDU MUST be set to zero)
#[test]
fn monitor_layout_pdu_source_is_zero() {
    let mut acceptor = multitransport_acceptor(None);
    let mut client_blocks = client_gcc_with_message_channel_and_multitransport(None);
    client_blocks
        .core
        .optional_data
        .early_capability_flags
        .get_or_insert_with(ClientEarlyCapabilityFlags::empty)
        .insert(ClientEarlyCapabilityFlags::SUPPORT_MONITOR_LAYOUT_PDU);
    drive_to_secure_settings_exchange(&mut acceptor, client_blocks);

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // CapabilitiesSendServer: sends Demand Active
    let mut output = WriteBuf::new();
    acceptor.step(&[], None, &mut output).unwrap(); // MonitorLayoutSend

    let indication = mcs::decode_send_data_indication(output.filled()).unwrap();
    let share_control = decode_share_control(indication).unwrap();
    let ShareControlPdu::Data(ShareDataHeader {
        share_data_pdu: ShareDataPdu::MonitorLayout(_),
        ..
    }) = share_control.pdu
    else {
        panic!("expected a Monitor Layout PDU");
    };
    assert_eq!(share_control.pdu_source, 0);
}

async fn accept_spnego_credssp(
    protocol: SecurityProtocol,
    username: &str,
    password: &str,
) -> (ConnectorResult<()>, SetupHandler, Vec<Vec<u8>>) {
    let mut acceptor = negotiated_acceptor(protocol);
    let public_key = vec![0x42; 32];
    let mut client = spnego_credssp_client(username, password, &public_key);
    let first_message = spnego_credssp_message(&mut client, TsRequest::default());
    let mut framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: [first_message].into(),
        spnego_credssp_client: Some(Box::new(client)),
        ..HandshakeStream::default()
    });
    let outgoing = Rc::clone(&framed.get_inner().0.outgoing);
    let mut handler = SetupHandler::default();
    let result = accept_credssp_with(
        &mut framed,
        &mut acceptor,
        &mut NoNetwork,
        "client.example.com".to_owned().into(),
        public_key,
        None,
        &mut handler,
    )
    .await;
    let outgoing = outgoing.borrow().clone();
    (result, handler, outgoing)
}

/// The mstsc client wraps NTLM in SPNEGO.
/// An acceptor that sent every client to the bare-NTLM server would fail this exchange.
#[tokio::test]
async fn credssp_authenticates_ntlm_wrapped_in_spnego() {
    // oracle: spec MS-CSSP §2.2.1.1, §2.2.1 and MS-RDPBCGR §2.2.10.2:
    // SPNEGO-wrapped NTLM delegates the client's credentials and ends with AUTHZ_SUCCESS (0).
    let (result, handler, outgoing) = accept_spnego_credssp(SecurityProtocol::HYBRID_EX, "alice", "secret").await;

    result.unwrap();
    assert_eq!(outgoing.last().unwrap(), &0u32.to_le_bytes());
    assert_eq!(handler.received.len(), 1);
    let received = &handler.received[0];
    assert_eq!(received.origin, ironrdp_acceptor::CredentialOrigin::CredSspDelegated);
    assert_eq!(received.credentials.username, "alice");
    assert_eq!(received.credentials.domain, None);
    assert_eq!(received.credentials.password, "secret");
}

#[tokio::test]
async fn credssp_preserves_explicit_domain_when_username_contains_at_sign() {
    // oracle: mutant discarding an explicit domain when the configured username contains '@'.
    let protocol = SecurityProtocol::HYBRID_EX;
    let mut acceptor = negotiated_acceptor_expecting(
        protocol,
        Some(Credentials {
            username: "alice@example.com".to_owned(),
            password: "secret".to_owned(),
            domain: Some("MicrosoftAccount".to_owned()),
        }),
    );
    let public_key = vec![0x42; 32];
    let (mut client, request) = CredsspSequence::init(
        ironrdp_connector::Credentials::UsernamePassword {
            username: "alice@example.com".to_owned(),
            password: "secret".to_owned(),
        },
        Some("MicrosoftAccount"),
        protocol,
        "server.example.com".to_owned().into(),
        public_key.clone(),
        None,
    )
    .unwrap();
    let mut framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: [client_credssp_message(&mut client, request)].into(),
        credssp_client: Some(client),
        ..HandshakeStream::default()
    });
    let mut handler = SetupHandler::default();
    accept_credssp_with(
        &mut framed,
        &mut acceptor,
        &mut NoNetwork,
        "client.example.com".to_owned().into(),
        public_key,
        None,
        &mut handler,
    )
    .await
    .unwrap();

    assert_eq!(handler.received.len(), 1);
    assert_eq!(handler.received[0].credentials.username, "alice@example.com");
    assert_eq!(
        handler.received[0].credentials.domain.as_deref(),
        Some("MicrosoftAccount")
    );
}

/// The sspi crate rejects an unknown user in the SPNEGO exchange with a different error than a wrong password.
/// Both must reach the client as STATUS_LOGON_FAILURE, so that it asks for new credentials
/// instead of reporting an internal error.
#[rstest::rstest]
#[case::wrong_password("alice", "wrong")]
#[case::unknown_user("bob", "secret")]
#[tokio::test]
async fn credssp_failed_spnego_logon_is_reported_as_logon_failure(#[case] username: &str, #[case] password: &str) {
    // oracle: spec MS-CSSP §2.2.1 and MS-ERREF §2.3.1: errorCode carries STATUS_LOGON_FAILURE for bad authentication information.
    // oracle: mutant reporting sspi's NoCredentials instead of STATUS_LOGON_FAILURE for an unknown SPNEGO user.
    let (result, handler, outgoing) = accept_spnego_credssp(SecurityProtocol::HYBRID, username, password).await;

    assert!(result.is_err(), "the logon should fail");
    assert!(handler.received.is_empty());
    let ts_request = TsRequest::from_buffer(outgoing.last().unwrap()).unwrap();
    assert_eq!(ts_request.error_code, Some(NStatusCode::LOGON_FAILURE));
}

#[cfg(windows)]
fn native_acceptor() -> Acceptor {
    let protocol = SecurityProtocol::HYBRID_EX;
    let mut acceptor = Acceptor::new(
        protocol,
        DesktopSize {
            width: 1024,
            height: 768,
        },
        Vec::new(),
        None,
    );
    acceptor.set_credssp_backend(ironrdp_acceptor::CredsspBackend::Windows);
    acceptor
        .step(&encode_connection_request(protocol), None, &mut WriteBuf::new())
        .unwrap();
    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap();
    acceptor.mark_security_upgrade_as_done();
    acceptor
}

#[cfg(windows)]
fn native_negotiate_message() -> Vec<u8> {
    let (mut client, request) = CredsspSequence::init(
        ironrdp_connector::Credentials::UsernamePassword {
            username: "unused-before-authentication".to_owned(),
            password: "unused-before-authentication".to_owned(),
        },
        None,
        SecurityProtocol::HYBRID_EX,
        "localhost".to_owned().into(),
        vec![0x42; 32],
        None,
    )
    .unwrap();
    client_credssp_message(&mut client, request)
}

#[cfg(windows)]
#[tokio::test]
async fn native_credssp_challenges_without_a_configured_identity() {
    // oracle: mutant requiring the fixed-credential identity before dispatching to Windows.
    let mut framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: [native_negotiate_message()].into(),
        ..HandshakeStream::default()
    });
    let outgoing = Rc::clone(&framed.get_inner().0.outgoing);
    let mut acceptor = native_acceptor();
    let mut handler = SetupHandler::default();
    let result = accept_credssp_with(
        &mut framed,
        &mut acceptor,
        &mut NoNetwork,
        "localhost".to_owned().into(),
        vec![0x42; 32],
        None,
        &mut handler,
    )
    .await;

    assert!(result.is_err(), "the client never answered the challenge");
    assert!(handler.received.is_empty());
    assert!(acceptor.received_credentials().is_none());
    let outgoing = outgoing.borrow();
    let challenge = TsRequest::from_buffer(outgoing.first().expect("Windows must issue a challenge")).unwrap();
    assert!(challenge.nego_tokens.is_some());
    assert_eq!(challenge.error_code, None);
    assert_eq!(outgoing.last().unwrap(), &5u32.to_le_bytes());
}

#[cfg(windows)]
#[rstest::rstest]
#[case::legacy_version(4, None)]
#[case::credentials_before_authentication(6, Some(vec![0; 32]))]
#[tokio::test]
async fn native_credssp_rejects_invalid_initial_requests(#[case] version: u32, #[case] auth_info: Option<Vec<u8>>) {
    // oracle: mutant accepting obsolete binding versions or credentials before Negotiate completes.
    let mut request = TsRequest::from_buffer(&native_negotiate_message()).unwrap();
    request.version = version;
    request.auth_info = auth_info;
    let mut message = Vec::new();
    request.encode_ts_request(&mut message).unwrap();
    let mut framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: [message].into(),
        ..HandshakeStream::default()
    });
    let outgoing = Rc::clone(&framed.get_inner().0.outgoing);
    let mut acceptor = native_acceptor();
    let mut handler = SetupHandler::default();
    let result = accept_credssp_with(
        &mut framed,
        &mut acceptor,
        &mut NoNetwork,
        "localhost".to_owned().into(),
        vec![0x42; 32],
        None,
        &mut handler,
    )
    .await;

    assert!(result.is_err());
    assert!(handler.received.is_empty());
    assert!(acceptor.received_credentials().is_none());
    assert!(
        outgoing
            .borrow()
            .iter()
            .filter(|message| message.len() != 4)
            .all(|message| TsRequest::from_buffer(message).unwrap().nego_tokens.is_none()),
        "invalid initial requests must not advance Negotiate"
    );
    if version == 4 {
        // oracle: spec MS-CSSP §3.1.5: unsupported version returns STATUS_NOT_SUPPORTED (0xC00000BB).
        let rejection = TsRequest::from_buffer(outgoing.borrow().first().unwrap()).unwrap();
        assert_eq!(rejection.error_code, Some(NStatusCode(0xc000_00bb)));
    }
    assert_eq!(outgoing.borrow().last().unwrap(), &5u32.to_le_bytes());
}

#[cfg(windows)]
fn native_test_identity() -> (String, String) {
    (
        std::env::var("IRONRDP_NATIVE_TEST_USERNAME").expect("a disposable local test account is required"),
        std::env::var("IRONRDP_NATIVE_TEST_PASSWORD").expect("the disposable account password is required"),
    )
}

#[cfg(windows)]
async fn native_test_exchange(
    username: String,
    password: String,
    client_public_key: Vec<u8>,
) -> (ConnectorResult<()>, SetupHandler) {
    let (mut client, request) = CredsspSequence::init(
        ironrdp_connector::Credentials::UsernamePassword { username, password },
        None,
        SecurityProtocol::HYBRID_EX,
        "localhost".to_owned().into(),
        client_public_key,
        None,
    )
    .unwrap();
    let mut framed = Framed::<HandshakeStream>::new(HandshakeStream {
        incoming: [client_credssp_message(&mut client, request)].into(),
        credssp_client: Some(client),
        ..HandshakeStream::default()
    });
    let mut handler = SetupHandler::default();
    let result = accept_credssp_with(
        &mut framed,
        &mut native_acceptor(),
        &mut NoNetwork,
        "localhost".to_owned().into(),
        vec![0x42; 32],
        None,
        &mut handler,
    )
    .await;
    (result, handler)
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires a disposable local Windows account in IRONRDP_NATIVE_TEST_USERNAME/PASSWORD"]
async fn native_credssp_delegates_authenticated_local_credentials() {
    // oracle: spec MS-CSSP §3.1.5, steps 4-5: verify the binding before accepting delegated credentials.
    let (username, password) = native_test_identity();
    let (result, handler) = native_test_exchange(username.clone(), password.clone(), vec![0x42; 32]).await;
    result.unwrap();
    assert_eq!(handler.received.len(), 1);
    assert_eq!(
        handler.received[0].origin,
        ironrdp_acceptor::CredentialOrigin::CredSspDelegated
    );
    let expected = Username::parse(&username).unwrap();
    let ironrdp_connector::sspi::UsernameParts::DownLevelLogonName(parts) = expected.parts() else {
        panic!("the fixture must use a local account");
    };
    assert_eq!(handler.received[0].credentials.username, parts.account_name());
    assert_eq!(
        handler.received[0].credentials.domain.as_deref(),
        parts.netbios_domain()
    );
    assert!(
        handler.received[0].credentials.password == password,
        "the delegated password changed"
    );
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires a disposable local Windows account in IRONRDP_NATIVE_TEST_USERNAME/PASSWORD"]
async fn native_credssp_rejects_wrong_local_password() {
    // oracle: spec MS-CSSP §3.1.5: authentication must complete successfully before delegation.
    let (username, password) = native_test_identity();
    let (result, handler) = native_test_exchange(username, format!("{password}-incorrect"), vec![0x42; 32]).await;
    assert!(result.is_err());
    assert!(handler.received.is_empty());
}

#[cfg(windows)]
#[tokio::test]
#[ignore = "requires a disposable local Windows account in IRONRDP_NATIVE_TEST_USERNAME/PASSWORD"]
async fn native_credssp_rejects_a_different_tls_public_key() {
    // oracle: spec MS-CSSP §3.1.5, step 4: reject a mismatched Client-To-Server binding hash.
    let (username, password) = native_test_identity();
    let (result, handler) = native_test_exchange(username, password, vec![0x24; 32]).await;
    assert!(result.is_err());
    assert!(handler.received.is_empty());
}
