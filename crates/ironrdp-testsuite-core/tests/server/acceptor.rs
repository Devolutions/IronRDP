use std::borrow::Cow;

use ironrdp_acceptor::Acceptor;
use ironrdp_connector::{DesktopSize, Sequence as _, Written, encode_x224_packet};
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_pdu::gcc::{ClientMessageChannelData, MultiTransportChannelData, MultiTransportFlags};
use ironrdp_pdu::mcs::{self, ConnectInitial};
use ironrdp_pdu::nego::{self, SecurityProtocol};
use ironrdp_pdu::rdp::client_info::CompressionType;
use ironrdp_pdu::rdp::finalization_messages::{ControlAction, ControlPdu, SynchronizePdu};
use ironrdp_pdu::rdp::headers::{
    BasicSecurityHeaderFlags, CompressionFlags, ShareControlHeader, ShareControlPdu, ShareDataHeader, ShareDataPdu,
    StreamPriority,
};
use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, MultitransportResponsePdu, RequestedProtocol};
use ironrdp_pdu::x224::{X224, X224Data};
use ironrdp_testsuite_core::gcc::CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS;
use ironrdp_testsuite_core::rdp::{CLIENT_DEMAND_ACTIVE_PDU_BUFFER, CLIENT_INFO_PDU_BUFFER};

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

fn encode_send_data_request(initiator_id: u16, channel_id: u16, user_data: &[u8]) -> Vec<u8> {
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

    // Join every channel the server expects: the user and I/O channels, plus
    // the message channel when one was negotiated. No other static channels
    // are requested by `client_blocks.network` in these tests.
    let mut to_join = vec![user_channel_id, io_channel_id];
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
    let client_info = encode_send_data_request(user_channel_id, io_channel_id, &CLIENT_INFO_PDU_BUFFER);
    acceptor.step(&client_info, None, &mut WriteBuf::new()).unwrap();

    (
        user_channel_id,
        io_channel_id,
        message_channel_id,
        server_multitransport,
    )
}

fn client_gcc_with_message_channel_and_multitransport(
    offer: Option<MultiTransportFlags>,
) -> ironrdp_pdu::gcc::ClientGccBlocks {
    let mut blocks = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    blocks.network = None;
    blocks.message_channel = Some(ClientMessageChannelData);
    blocks.multi_transport_channel = offer.map(|flags| MultiTransportChannelData { flags });
    blocks
}

/// Builds an `Acceptor` for the multitransport tests below, at a common
/// 1920x1080 desktop size with no static channels or credentials. `offer` is
/// passed to `set_multitransport_offer` when `Some`; pass `None` to exercise
/// the default-disabled path.
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
    if let Some(offer) = offer {
        acceptor.set_multitransport_offer(Some(offer));
    }
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
    drive_to_secure_settings_exchange(&mut acceptor, client_blocks);

    acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // LicensingExchange
    let written = acceptor.step(&[], None, &mut WriteBuf::new()).unwrap(); // MultitransportBootstrapping
    assert!(matches!(written, Written::Nothing));
    assert!(acceptor.multitransport_request().is_none());
    assert_eq!(acceptor.multitransport_soft_sync_negotiated(), None);
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
