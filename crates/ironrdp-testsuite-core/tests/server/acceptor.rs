use ironrdp_acceptor::Acceptor;
use ironrdp_connector::{DesktopSize, Sequence as _, Written, encode_x224_packet};
use ironrdp_core::{WriteBuf, decode};
use ironrdp_pdu::gcc::ClientMessageChannelData;
use ironrdp_pdu::mcs::{self, ConnectInitial};
use ironrdp_pdu::nego::{self, SecurityProtocol};
use ironrdp_pdu::x224::{X224, X224Data};
use ironrdp_testsuite_core::gcc::CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS;

/// Build a minimal ConnectionRequest with the given protocols and encode it.
fn encode_connection_request(protocol: SecurityProtocol) -> Vec<u8> {
    let request = nego::ConnectionRequest {
        nego_data: None,
        flags: nego::RequestFlags::empty(),
        protocol,
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
    let written = acceptor.step(&request_bytes, &mut output).unwrap();
    assert!(matches!(written, Written::Nothing));

    // Step 2: acceptor tries to send confirm, finds no common protocol
    let mut output = WriteBuf::new();
    let result = acceptor.step(&[], &mut output);

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
    acceptor.step(&request_bytes, &mut output).unwrap();

    let mut output = WriteBuf::new();
    let written = acceptor.step(&[], &mut output).unwrap();
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
/// Message Channel Data. With no static virtual channels the ID is allocated
/// right after the I/O channel (1003), i.e. 1004.
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
    acceptor.step(&request_bytes, &mut WriteBuf::new()).unwrap();
    acceptor.step(&[], &mut WriteBuf::new()).unwrap();
    acceptor.mark_security_upgrade_as_done();

    // Client GCC with the message channel block and no network channels, so the
    // allocated ID is deterministic.
    let mut blocks = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    blocks.network = None;
    blocks.message_channel = Some(ClientMessageChannelData);
    let connect_initial = ConnectInitial::with_gcc_blocks(blocks).unwrap();
    let mut initial_buf = WriteBuf::new();
    encode_x224_packet(&connect_initial, &mut initial_buf).unwrap();

    acceptor.step(initial_buf.filled(), &mut WriteBuf::new()).unwrap();

    let mut output = WriteBuf::new();
    acceptor.step(&[], &mut output).unwrap();

    let payload = decode::<X224<X224Data<'_>>>(output.filled()).unwrap().0;
    let response = decode::<mcs::ConnectResponse>(payload.data.as_ref()).unwrap();
    let server_blocks = response.conference_create_response.gcc_blocks();

    let message_channel = server_blocks
        .message_channel
        .as_ref()
        .expect("acceptor must advertise Server Message Channel Data");
    assert_eq!(message_channel.mcs_message_channel_id, 1004);
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
    acceptor.step(&request_bytes, &mut output).unwrap();

    let mut output = WriteBuf::new();
    let result = acceptor.step(&[], &mut output);
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
