#![allow(unused_crate_dependencies)]

use ironrdp_mstsgu::rpc::{
    DEFAULT_FRAGMENT_SIZE, MAX_PENDING_RPC_FRAGMENTS, PFC_FIRST_FRAG, PFC_LAST_FRAG, PFC_SUPPORT_HEADER_SIGN,
    PTYPE_BIND, PTYPE_BIND_ACK, PTYPE_BIND_NAK, PTYPE_FAULT, PTYPE_REQUEST, PTYPE_RESPONSE, PTYPE_RPC_AUTH_3,
    RPC_AUTH_LEVEL_PACKET_INTEGRITY, RPC_COMMON_HEADER_SIZE, RPC_DREP_LITTLE_ENDIAN, RPC_SECURITY_TRAILER_SIZE,
    RPC_VERSION, RPC_VERSION_MINOR, RpcAuthenticatedResponseReassembler, RpcAuthenticationInfo, RpcCommonHeader,
    RpcFault, RpcFragmentSizes, RpcNtlmAuth, RpcPduError, RpcPduStream, RpcPresentationContext, RpcReassembledResponse,
    RpcResponse, RpcResponseReassembler, RpcSyntaxIdentifier, RpcSyntaxVersion, decode_rpc_authenticated_fragment,
    decode_rpc_bind_ack, decode_rpc_bind_ack_with_ntlm_auth, decode_rpc_bind_nak, decode_rpc_fault,
    decode_rpc_fault_for_context, decode_rpc_response, decode_rpc_response_for_context, decode_rpc_response_fragment,
    encode_rpc_auth_3, encode_rpc_bind, encode_rpc_bind_with_ntlm_auth, encode_rpc_fault, encode_rpc_request_fragments,
    encode_rpc_response, encode_rpc_response_fragment, prepare_rpc_authenticated_bind, prepare_rpc_authenticated_pdu,
    prepare_rpc_authenticated_request_fragments,
};
use uuid::Uuid;

#[test]
fn syntax_version_and_fragment_sizes_round_trip_accessors() {
    let version = RpcSyntaxVersion::new(2, 0);
    assert_eq!(version.major(), 2);
    assert_eq!(version.minor(), 0);

    let sizes = RpcFragmentSizes::new(32, 64).expect("valid maxima");
    assert_eq!(sizes.max_xmit(), 32);
    assert_eq!(sizes.max_recv(), 64);
    assert_eq!(RpcFragmentSizes::DEFAULT.max_xmit(), DEFAULT_FRAGMENT_SIZE);
    let too_small = u16::try_from(RPC_COMMON_HEADER_SIZE).expect("header size fits") - 1;
    assert_eq!(
        RpcFragmentSizes::new(too_small, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::InvalidFragmentSize { maximum: too_small })
    );
}

#[test]
fn common_header_round_trips_response_metadata() {
    let pdu = encode_rpc_response(0x7856_3412, &[1, 2, 3]).expect("valid response");
    let header = RpcCommonHeader::decode(&pdu).expect("complete fragment");
    assert_eq!(header.ptype(), PTYPE_RESPONSE);
    assert_eq!(header.pfc_flags(), PFC_FIRST_FRAG | PFC_LAST_FRAG);
    assert_eq!(header.auth_length(), 0);
    assert_eq!(header.call_id(), 0x7856_3412);
    assert_eq!(usize::from(header.fragment_length()), pdu.len());
    assert_eq!(&pdu[..2], &[RPC_VERSION, RPC_VERSION_MINOR]);
    assert_eq!(&pdu[4..8], &RPC_DREP_LITTLE_ENDIAN);
}

#[test]
fn common_header_rejects_invalid_versions_drep_and_fragment_lengths() {
    assert_eq!(
        RpcCommonHeader::decode(&[0; RPC_COMMON_HEADER_SIZE - 1]),
        Err(RpcPduError::Truncated {
            actual: RPC_COMMON_HEADER_SIZE - 1,
            required: RPC_COMMON_HEADER_SIZE
        })
    );

    let mut pdu = encode_rpc_response(1, &[]).expect("valid response");
    pdu[0] = 4;
    assert_eq!(
        RpcCommonHeader::decode(&pdu),
        Err(RpcPduError::UnsupportedVersion { major: 4, minor: 0 })
    );

    let mut pdu = encode_rpc_response(1, &[]).expect("valid response");
    pdu[4] = 0;
    assert_eq!(
        RpcCommonHeader::decode(&pdu),
        Err(RpcPduError::UnsupportedDataRepresentation { value: [0, 0, 0, 0] })
    );

    let mut pdu = encode_rpc_response(1, &[]).expect("valid response");
    pdu[8..10].copy_from_slice(&15u16.to_le_bytes());
    assert_eq!(
        RpcCommonHeader::decode(&pdu),
        Err(RpcPduError::InvalidFragmentLength { fragment_length: 15 })
    );

    let mut pdu = encode_rpc_response(1, &[1, 2, 3]).expect("valid response");
    let claimed = u16::try_from(pdu.len()).expect("test length fits");
    pdu.truncate(RPC_COMMON_HEADER_SIZE);
    assert_eq!(
        RpcCommonHeader::decode(&pdu),
        Err(RpcPduError::IncompleteFragment {
            actual: RPC_COMMON_HEADER_SIZE,
            fragment_length: claimed,
        })
    );
}

#[test]
fn response_and_fault_vectors_round_trip_metadata_and_stubs() {
    let response = encode_rpc_response(0x7856_3412, &[1, 2, 3]).expect("valid response");
    assert_eq!(
        decode_rpc_response(&response, DEFAULT_FRAGMENT_SIZE).expect("valid response"),
        RpcResponse {
            call_id: 0x7856_3412,
            pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
            alloc_hint: 3,
            cancel_count: 0,
            reserved: 0,
            stub: &[1, 2, 3],
        }
    );

    let first = encode_rpc_response_fragment(RpcResponse {
        call_id: 0x7856_3412,
        pfc_flags: PFC_FIRST_FRAG,
        alloc_hint: 3,
        cancel_count: 0,
        reserved: 0,
        stub: &[1, 2, 3],
    })
    .expect("valid response fragment");
    assert_eq!(
        decode_rpc_response_fragment(&first, DEFAULT_FRAGMENT_SIZE).expect("valid response fragment"),
        RpcResponse {
            call_id: 0x7856_3412,
            pfc_flags: PFC_FIRST_FRAG,
            alloc_hint: 3,
            cancel_count: 0,
            reserved: 0,
            stub: &[1, 2, 3],
        }
    );

    let fault = RpcFault {
        call_id: 0x0102_0304,
        pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
        alloc_hint: 2,
        cancel_count: 0,
        reserved: 0,
        status: 0xdead_beef,
        reserved2: 1,
        stub: &[0x12, 0x34],
    };
    let mut encoded = encode_rpc_fault(fault).expect("valid fault");
    assert_eq!(encoded[2], PTYPE_FAULT);
    assert_eq!(&encoded[RPC_COMMON_HEADER_SIZE + 12..][..4], &0u32.to_le_bytes());
    encoded[3] |= 0x04;
    encoded[RPC_COMMON_HEADER_SIZE + 12..][..4].copy_from_slice(&fault.reserved2.to_le_bytes());
    assert_eq!(
        decode_rpc_fault(&encoded, DEFAULT_FRAGMENT_SIZE).expect("valid fault"),
        RpcFault {
            pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG | 0x04,
            ..fault
        }
    );
}

#[test]
fn rpc_decoders_reject_unsupported_fragments_auth_and_untrusted_lengths() {
    let mut response = encode_rpc_response(1, &[]).expect("valid response");
    response[3] = PFC_FIRST_FRAG;
    assert_eq!(
        decode_rpc_response(&response, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::FragmentedPduUnsupported { flags: PFC_FIRST_FRAG })
    );

    response[3] = PFC_FIRST_FRAG | PFC_LAST_FRAG;
    response[10..12].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(
        decode_rpc_response(&response, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::AuthenticationUnsupported { auth_length: 1 })
    );

    response[10..12].copy_from_slice(&0u16.to_le_bytes());
    response[16..20].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        decode_rpc_response(&response, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::InvalidAllocHint {
            alloc_hint: 1,
            stub_length: 0
        })
    );

    let fault = encode_rpc_fault(RpcFault {
        call_id: 1,
        pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
        alloc_hint: 0,
        cancel_count: 0,
        reserved: 0,
        status: 0,
        reserved2: 0,
        stub: &[],
    })
    .expect("valid fault");
    assert_eq!(
        decode_rpc_response(&fault, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::UnexpectedPduType {
            expected: PTYPE_RESPONSE,
            actual: PTYPE_FAULT,
        })
    );
}

#[test]
fn pdu_stream_yields_complete_fragments_without_losing_partial_data() {
    let first = encode_rpc_response(1, &[1, 2]).expect("valid response");
    let second = encode_rpc_response(2, &[3]).expect("valid response");
    let mut stream = RpcPduStream::new(RpcFragmentSizes::DEFAULT.max_recv()).expect("valid maximum");

    stream
        .push(&first[..RPC_COMMON_HEADER_SIZE - 1])
        .expect("bounded partial fragment");
    assert_eq!(stream.next_fragment(), Ok(None));

    stream
        .push(&first[RPC_COMMON_HEADER_SIZE - 1..])
        .expect("bounded first fragment");
    stream.push(&second).expect("bounded second fragment");
    assert_eq!(stream.next_fragment(), Ok(Some(first)));
    assert_eq!(stream.next_fragment(), Ok(Some(second)));
    assert_eq!(stream.next_fragment(), Ok(None));
}

#[test]
fn pdu_stream_rejects_oversized_and_invalid_fragments_before_buffering_them() {
    let mut stream = RpcPduStream::new(32).expect("valid maximum");
    let oversized = encode_rpc_response(1, &[0; 17]).expect("valid response");
    stream
        .push(&oversized[..RPC_COMMON_HEADER_SIZE])
        .expect("bounded oversized header");
    assert_eq!(
        stream.next_fragment(),
        Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length: u16::try_from(oversized.len()).expect("test length fits in u16"),
            maximum: 32,
        })
    );

    let mut stream = RpcPduStream::new(RpcFragmentSizes::DEFAULT.max_recv()).expect("valid maximum");
    stream
        .push(&[4; RPC_COMMON_HEADER_SIZE])
        .expect("bounded invalid header");
    assert_eq!(
        stream.next_fragment(),
        Err(RpcPduError::UnsupportedVersion { major: 4, minor: 4 })
    );

    let too_small = u16::try_from(RPC_COMMON_HEADER_SIZE).expect("header size fits") - 1;
    assert_eq!(
        RpcPduStream::new(too_small).err(),
        Some(RpcPduError::InvalidFragmentSize { maximum: too_small })
    );
}

#[test]
fn pdu_stream_caps_pending_bytes() {
    let mut stream = RpcPduStream::new(32).expect("valid maximum");
    assert_eq!(
        stream.push(&[0; 32 * MAX_PENDING_RPC_FRAGMENTS + 1]),
        Err(RpcPduError::PendingBytesExceedMaximum {
            actual: 32 * MAX_PENDING_RPC_FRAGMENTS + 1,
            maximum: 32 * MAX_PENDING_RPC_FRAGMENTS,
        })
    );
}

#[test]
fn response_reassembler_requires_ordered_fragments_and_bounds_the_stub() {
    let mut reassembler = RpcResponseReassembler::new(5);
    let first = RpcResponse {
        call_id: 7,
        pfc_flags: PFC_FIRST_FRAG,
        alloc_hint: 5,
        cancel_count: 0,
        reserved: 0,
        stub: &[1, 2],
    };
    assert_eq!(reassembler.push(first), Ok(None));
    let last = RpcResponse {
        call_id: 7,
        pfc_flags: PFC_LAST_FRAG,
        alloc_hint: 3,
        cancel_count: 0,
        reserved: 0,
        stub: &[3, 4, 5],
    };
    assert_eq!(
        reassembler.push(last),
        Ok(Some(RpcReassembledResponse {
            call_id: 7,
            cancel_count: 0,
            reserved: 0,
            stub: vec![1, 2, 3, 4, 5],
        }))
    );

    let unexpected = RpcResponse {
        call_id: 8,
        pfc_flags: PFC_LAST_FRAG,
        alloc_hint: 1,
        cancel_count: 0,
        reserved: 0,
        stub: &[1],
    };
    assert_eq!(
        reassembler.push(unexpected),
        Err(RpcPduError::UnexpectedResponseFragment { flags: PFC_LAST_FRAG })
    );

    let oversized = RpcResponse {
        call_id: 8,
        pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
        alloc_hint: 6,
        cancel_count: 0,
        reserved: 0,
        stub: &[1, 2, 3, 4, 5, 6],
    };
    assert_eq!(
        reassembler.push(oversized),
        Err(RpcPduError::ResponseStubTooLarge { actual: 6, maximum: 5 })
    );
}

#[test]
fn response_reassembly_round_trips_through_the_pdu_stream() {
    let first = encode_rpc_response_fragment(RpcResponse {
        call_id: 9,
        pfc_flags: PFC_FIRST_FRAG,
        alloc_hint: 5,
        cancel_count: 1,
        reserved: 2,
        stub: &[1, 2],
    })
    .expect("first fragment");
    assert_eq!(first[RPC_COMMON_HEADER_SIZE + 7], 0);
    let mut first = first;
    first[RPC_COMMON_HEADER_SIZE + 7] = 2;
    let last = encode_rpc_response_fragment(RpcResponse {
        call_id: 9,
        pfc_flags: PFC_LAST_FRAG,
        alloc_hint: 3,
        cancel_count: 0,
        reserved: 0,
        stub: &[3, 4, 5],
    })
    .expect("last fragment");

    let mut stream = RpcPduStream::new(DEFAULT_FRAGMENT_SIZE).expect("valid maximum");
    stream.push(&first).expect("first fragment");
    stream.push(&last).expect("last fragment");

    let mut reassembler = RpcResponseReassembler::new(5);
    let first_bytes = stream.next_fragment().expect("first").expect("complete");
    let first = decode_rpc_response_fragment(&first_bytes, DEFAULT_FRAGMENT_SIZE).expect("first fragment");
    assert!(!first.is_last_fragment());
    assert_eq!(reassembler.push(first), Ok(None));

    let last_bytes = stream.next_fragment().expect("last").expect("complete");
    let last = decode_rpc_response_fragment(&last_bytes, DEFAULT_FRAGMENT_SIZE).expect("last fragment");
    assert!(last.is_last_fragment());
    assert_eq!(
        reassembler.push(last),
        Ok(Some(RpcReassembledResponse {
            call_id: 9,
            cancel_count: 0,
            reserved: 2,
            stub: vec![1, 2, 3, 4, 5],
        }))
    );
}

#[test]
fn response_reassembler_rejects_call_id_mismatch_and_invalid_alloc_hint() {
    let mut reassembler = RpcResponseReassembler::new(10);
    reassembler
        .push(RpcResponse {
            call_id: 7,
            pfc_flags: PFC_FIRST_FRAG,
            alloc_hint: 2,
            cancel_count: 0,
            reserved: 0,
            stub: &[1, 2],
        })
        .expect("first fragment");
    assert_eq!(
        reassembler.push(RpcResponse {
            call_id: 8,
            pfc_flags: PFC_LAST_FRAG,
            alloc_hint: 1,
            cancel_count: 0,
            reserved: 0,
            stub: &[3],
        }),
        Err(RpcPduError::ResponseFragmentCallId { expected: 7, actual: 8 })
    );

    let mut reassembler = RpcResponseReassembler::new(10);
    reassembler
        .push(RpcResponse {
            call_id: 7,
            pfc_flags: PFC_FIRST_FRAG,
            alloc_hint: 9,
            cancel_count: 0,
            reserved: 0,
            stub: &[1, 2],
        })
        .expect("first fragment");
    assert_eq!(
        reassembler.push(RpcResponse {
            call_id: 7,
            pfc_flags: PFC_LAST_FRAG,
            alloc_hint: 1,
            cancel_count: 0,
            reserved: 0,
            stub: &[3],
        }),
        Err(RpcPduError::InvalidAllocHint {
            alloc_hint: 9,
            stub_length: 3
        })
    );
}

#[test]
fn response_reassembler_bounds_hints_and_recovers_from_an_oversized_first_fragment() {
    let mut reassembler = RpcResponseReassembler::new(usize::MAX);
    assert_eq!(
        reassembler.push(RpcResponse {
            call_id: 6,
            pfc_flags: PFC_FIRST_FRAG,
            alloc_hint: u32::MAX,
            cancel_count: 0,
            reserved: 0,
            stub: &[],
        }),
        Err(RpcPduError::ResponseStubTooLarge {
            actual: usize::try_from(u32::MAX).expect("u32 fits in usize"),
            maximum: 0x7fff_ffff,
        })
    );

    let mut reassembler = RpcResponseReassembler::new(5);
    assert_eq!(
        reassembler.push(RpcResponse {
            call_id: 7,
            pfc_flags: PFC_FIRST_FRAG,
            alloc_hint: 6,
            cancel_count: 0,
            reserved: 0,
            stub: &[],
        }),
        Err(RpcPduError::ResponseStubTooLarge { actual: 6, maximum: 5 })
    );
    assert_eq!(
        reassembler.push(RpcResponse {
            call_id: 8,
            pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
            alloc_hint: 1,
            cancel_count: 0,
            reserved: 0,
            stub: &[1],
        }),
        Ok(Some(RpcReassembledResponse {
            call_id: 8,
            cancel_count: 0,
            reserved: 0,
            stub: vec![1],
        }))
    );

    let mut reassembler = RpcResponseReassembler::new(1);
    for fragment_index in 0..MAX_PENDING_RPC_FRAGMENTS * 2 {
        assert_eq!(
            reassembler.push(RpcResponse {
                call_id: 9,
                pfc_flags: if fragment_index == 0 { PFC_FIRST_FRAG } else { 0 },
                alloc_hint: 1,
                cancel_count: 0,
                reserved: 0,
                stub: &[],
            }),
            Ok(None)
        );
    }

    assert_eq!(
        reassembler.push(RpcResponse {
            call_id: 9,
            pfc_flags: PFC_LAST_FRAG,
            alloc_hint: 1,
            cancel_count: 0,
            reserved: 0,
            stub: &[1],
        }),
        Ok(Some(RpcReassembledResponse {
            call_id: 9,
            cancel_count: 0,
            reserved: 0,
            stub: vec![1],
        }))
    );
}

#[test]
fn bind_and_request_codecs_match_connection_oriented_rpc_wire_layouts() {
    let abstract_syntax = RpcSyntaxIdentifier::new(
        Uuid::from_u128(0x00112233_4455_6677_8899_aabbccddeeff),
        RpcSyntaxVersion::new(1, 3),
    );
    let transfer_syntax = RpcSyntaxIdentifier::new(
        Uuid::from_u128(0x8a885d04_1ceb_11c9_9fe8_08002b104860),
        RpcSyntaxVersion::new(2, 0),
    );
    let presentation_context = RpcPresentationContext {
        context_id: 7,
        abstract_syntax,
        transfer_syntaxes: &[transfer_syntax],
    };
    let bind = encode_rpc_bind(0x7856_3412, RpcFragmentSizes::DEFAULT, 0, &[presentation_context]).expect("valid bind");
    assert_eq!(
        bind,
        [
            5,
            0,
            PTYPE_BIND,
            PFC_FIRST_FRAG | PFC_LAST_FRAG,
            0x10,
            0,
            0,
            0,
            72,
            0,
            0,
            0,
            0x12,
            0x34,
            0x56,
            0x78,
            0xb8,
            0x10,
            0xb8,
            0x10,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            7,
            0,
            1,
            0,
            0x33,
            0x22,
            0x11,
            0,
            0x55,
            0x44,
            0x77,
            0x66,
            0x88,
            0x99,
            0xaa,
            0xbb,
            0xcc,
            0xdd,
            0xee,
            0xff,
            1,
            0,
            3,
            0,
            4,
            0x5d,
            0x88,
            0x8a,
            0xeb,
            0x1c,
            0xc9,
            0x11,
            0x9f,
            0xe8,
            8,
            0,
            0x2b,
            0x10,
            0x48,
            0x60,
            2,
            0,
            0,
            0,
        ]
    );
    assert_eq!(
        encode_rpc_bind(1, RpcFragmentSizes::DEFAULT, 0, &[]),
        Err(RpcPduError::EmptyPresentationContexts)
    );
    assert_eq!(
        encode_rpc_bind(
            1,
            RpcFragmentSizes::DEFAULT,
            0,
            &[presentation_context, presentation_context],
        ),
        Err(RpcPduError::DuplicatePresentationContext { context_id: 7 })
    );

    let bind_ack = [
        5,
        0,
        PTYPE_BIND_ACK,
        PFC_FIRST_FRAG | PFC_LAST_FRAG,
        0x10,
        0,
        0,
        0,
        60,
        0,
        0,
        0,
        0x12,
        0x34,
        0x56,
        0x78,
        0xb8,
        0x10,
        0xb8,
        0x10,
        0xef,
        0xbe,
        0xad,
        0xde,
        3,
        0,
        b'1',
        b'3',
        b'5',
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        4,
        0x5d,
        0x88,
        0x8a,
        0xeb,
        0x1c,
        0xc9,
        0x11,
        0x9f,
        0xe8,
        8,
        0,
        0x2b,
        0x10,
        0x48,
        0x60,
        2,
        0,
        0,
        0,
    ];
    let offered_fragment_sizes = RpcFragmentSizes::new(0x1000, 0x0a00).expect("valid offered maxima");
    let mut bind_ack = bind_ack;
    bind_ack[16..18].copy_from_slice(&0x0c00u16.to_le_bytes());
    bind_ack[18..20].copy_from_slice(&0x0e00u16.to_le_bytes());
    let ack = decode_rpc_bind_ack(&bind_ack, offered_fragment_sizes).expect("valid bind acknowledgement");
    assert_eq!(ack.call_id, 0x7856_3412);
    assert_eq!(
        ack.fragment_sizes,
        RpcFragmentSizes::new(0x0e00, 0x0a00).expect("valid negotiated maxima")
    );
    assert_eq!(ack.association_group_id, 0xdead_beef);
    assert_eq!(ack.secondary_address, b"135");
    assert_eq!(ack.results.len(), 1);
    assert_eq!(ack.results[0].result, 0);
    assert_eq!(ack.results[0].reason, 0);
    assert_eq!(ack.results[0].transfer_syntax, transfer_syntax);
    let mut authenticated_bind_ack = bind_ack;
    authenticated_bind_ack[10..12].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(
        decode_rpc_bind_ack(&authenticated_bind_ack, RpcFragmentSizes::DEFAULT),
        Err(RpcPduError::AuthenticationUnsupported { auth_length: 1 })
    );
    let mut truncated_bind_ack = bind_ack.to_vec();
    truncated_bind_ack.pop();
    truncated_bind_ack[8..10].copy_from_slice(&59u16.to_le_bytes());
    assert_eq!(
        decode_rpc_bind_ack(&truncated_bind_ack, RpcFragmentSizes::DEFAULT),
        Err(RpcPduError::InvalidBindAckLength {
            actual: 43,
            expected: 44,
        })
    );

    let response = [
        5,
        0,
        PTYPE_RESPONSE,
        PFC_FIRST_FRAG | PFC_LAST_FRAG,
        0x10,
        0,
        0,
        0,
        27,
        0,
        0,
        0,
        0x12,
        0x34,
        0x56,
        0x78,
        3,
        0,
        0,
        0,
        7,
        0,
        0,
        0,
        1,
        2,
        3,
    ];
    assert_eq!(
        decode_rpc_response_for_context(&response, DEFAULT_FRAGMENT_SIZE, 7).expect("valid response"),
        RpcResponse {
            call_id: 0x7856_3412,
            pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
            alloc_hint: 3,
            cancel_count: 0,
            reserved: 0,
            stub: &[1, 2, 3],
        }
    );
}

#[test]
fn authenticated_pdu_vector_exposes_exact_security_segments() {
    let authentication = RpcAuthenticationInfo {
        auth_type: 0x0a,
        auth_level: 5,
        auth_context_id: 0x7856_3412,
        verifier_length: 3,
    };
    let prepared = prepare_rpc_authenticated_pdu(
        PTYPE_REQUEST,
        PFC_FIRST_FRAG | PFC_LAST_FRAG,
        0x0102_0304,
        &[0xaa, 0xbb],
        authentication,
    )
    .expect("authenticated request");
    assert_eq!(prepared.fragment_length(), 43);
    assert_eq!(prepared.body(), &[0xaa, 0xbb]);
    let segments = prepared.authentication_segments();
    assert_eq!(segments.header.len(), RPC_COMMON_HEADER_SIZE);
    assert_eq!(segments.body, &[0xaa, 0xbb, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(segments.security_trailer, &[0x0a, 5, 14, 0, 0x12, 0x34, 0x56, 0x78]);

    let pdu = prepared.finish(&[0xf1, 0xf2, 0xf3]).expect("reserved verifier length");
    assert_eq!(
        pdu,
        [
            5,
            0,
            PTYPE_REQUEST,
            PFC_FIRST_FRAG | PFC_LAST_FRAG,
            0x10,
            0,
            0,
            0,
            43,
            0,
            3,
            0,
            4,
            3,
            2,
            1,
            0xaa,
            0xbb,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0x0a,
            5,
            14,
            0,
            0x12,
            0x34,
            0x56,
            0x78,
            0xf1,
            0xf2,
            0xf3,
        ]
    );

    let fragment = decode_rpc_authenticated_fragment(&pdu, DEFAULT_FRAGMENT_SIZE).expect("authenticated fragment");
    assert_eq!(fragment.verifier(), &[0xf1, 0xf2, 0xf3]);
    let debug = format!("{fragment:?}");
    assert!(!debug.contains("170"));
    assert!(!debug.contains("187"));
    let fragment = fragment
        .verify_with(|segments, verifier| {
            assert_eq!(segments.body, &[0xaa, 0xbb, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            assert_eq!(segments.security_trailer, &[0x0a, 5, 14, 0, 0x12, 0x34, 0x56, 0x78]);
            assert_eq!(verifier, &[0xf1, 0xf2, 0xf3]);
            Ok::<(), ()>(())
        })
        .expect("caller accepts verifier");
    assert_eq!(fragment.body(), &[0xaa, 0xbb]);
    assert_eq!(fragment.authentication_padding(), &[0; 14]);
    assert_eq!(
        fragment.security_trailer(),
        ironrdp_mstsgu::rpc::RpcSecurityTrailer {
            auth_type: 0x0a,
            auth_level: 5,
            auth_pad_length: 14,
            auth_reserved: 0,
            auth_context_id: 0x7856_3412,
        }
    );
}

#[test]
fn authenticated_bind_sets_header_signing_and_obeys_fragment_maximum() {
    let abstract_syntax = RpcSyntaxIdentifier::new(
        Uuid::from_u128(0x00112233_4455_6677_8899_aabbccddeeff),
        RpcSyntaxVersion::new(1, 3),
    );
    let transfer_syntax = RpcSyntaxIdentifier::new(
        Uuid::from_u128(0x8a885d04_1ceb_11c9_9fe8_08002b104860),
        RpcSyntaxVersion::new(2, 0),
    );
    let presentation_context = RpcPresentationContext {
        context_id: 7,
        abstract_syntax,
        transfer_syntaxes: &[transfer_syntax],
    };
    let authentication = RpcAuthenticationInfo {
        auth_type: 0x0a,
        auth_level: 5,
        auth_context_id: 1,
        verifier_length: 4,
    };
    let prepared = prepare_rpc_authenticated_bind(
        1,
        RpcFragmentSizes::DEFAULT,
        0,
        &[presentation_context],
        true,
        authentication,
    )
    .expect("authenticated bind");
    assert_eq!(prepared.fragment_length(), 92);
    let pdu = prepared.finish(&[0; 4]).expect("reserved verifier length");
    assert_eq!(
        RpcCommonHeader::decode(&pdu).expect("complete bind").pfc_flags(),
        PFC_FIRST_FRAG | PFC_LAST_FRAG | PFC_SUPPORT_HEADER_SIGN
    );
    let bind_ack = prepare_rpc_authenticated_pdu(
        PTYPE_BIND_ACK,
        PFC_FIRST_FRAG | PFC_LAST_FRAG,
        1,
        &[0x00, 0x10, 0x00, 0x0a, 0xef, 0xbe, 0xad, 0xde, 0, 0, 0, 0, 0, 0, 0, 0],
        authentication,
    )
    .expect("authenticated bind acknowledgement")
    .finish(&[0; 4])
    .expect("reserved verifier length");
    let bind_ack = decode_rpc_authenticated_fragment(&bind_ack, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated bind acknowledgement")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier")
        .decode_bind_ack(RpcFragmentSizes::new(0x1000, 0x0a00).expect("valid offered maxima"))
        .expect("decoded bind acknowledgement");
    assert_eq!(bind_ack.association_group_id, 0xdead_beef);
    assert_eq!(bind_ack.results, []);
    assert_eq!(
        prepare_rpc_authenticated_bind(
            1,
            RpcFragmentSizes::new(80, 80).expect("valid maxima"),
            0,
            &[presentation_context],
            false,
            authentication,
        ),
        Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length: 92,
            maximum: 80,
        })
    );
}

#[test]
fn authenticated_request_fragments_reserve_and_exclude_security_data_per_fragment() {
    let authentication = RpcAuthenticationInfo {
        auth_type: 0x0a,
        auth_level: 5,
        auth_context_id: 1,
        verifier_length: 4,
    };
    let fragment_sizes = RpcFragmentSizes::new(52, 52).expect("valid maxima");
    let prepared = prepare_rpc_authenticated_request_fragments(
        0x7856_3412,
        7,
        3,
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
        fragment_sizes,
        authentication,
    )
    .expect("authenticated request fragments");
    assert_eq!(prepared.len(), 3);

    let fragments: Vec<_> = prepared
        .into_iter()
        .map(|prepared| {
            assert_eq!(prepared.fragment_length(), 44);
            assert_eq!(
                prepared.authentication_segments().header.len() + prepared.authentication_segments().body.len(),
                32
            );
            assert_eq!(
                prepared.authentication_segments().security_trailer.len(),
                RPC_SECURITY_TRAILER_SIZE
            );
            prepared
                .finish(&[0xde, 0xad, 0xbe, 0xef])
                .expect("reserved verifier length")
        })
        .collect();

    let expected_flags = [PFC_FIRST_FRAG, 0, PFC_LAST_FRAG];
    let expected_alloc_hints = [17u32, 9, 1];
    let expected_stubs: [&[u8]; 3] = [&[1, 2, 3, 4, 5, 6, 7, 8], &[9, 10, 11, 12, 13, 14, 15, 16], &[17]];
    for ((fragment, flags), (alloc_hint, stub)) in fragments
        .iter()
        .zip(expected_flags)
        .zip(expected_alloc_hints.into_iter().zip(expected_stubs))
    {
        let authenticated =
            decode_rpc_authenticated_fragment(fragment, fragment_sizes.max_xmit()).expect("authenticated fragment");
        assert_eq!(authenticated.header().pfc_flags(), flags);
        assert_eq!(authenticated.verifier(), &[0xde, 0xad, 0xbe, 0xef]);
        let authenticated = authenticated
            .verify_with(|_, verifier| {
                assert_eq!(verifier, &[0xde, 0xad, 0xbe, 0xef]);
                Ok::<(), ()>(())
            })
            .expect("caller accepts verifier");
        assert_eq!(&authenticated.body()[..4], &alloc_hint.to_le_bytes());
        assert_eq!(&authenticated.body()[4..6], &7u16.to_le_bytes());
        assert_eq!(&authenticated.body()[6..8], &3u16.to_le_bytes());
        assert_eq!(&authenticated.body()[8..], stub);
    }

    assert_eq!(
        prepare_rpc_authenticated_request_fragments(
            1,
            0,
            0,
            &[],
            RpcFragmentSizes::new(43, 43).expect("valid maxima"),
            authentication
        ),
        Err(RpcPduError::FragmentTooSmall {
            maximum: 43,
            required: 44,
        })
    );
    assert_eq!(
        prepare_rpc_authenticated_request_fragments(
            1,
            0,
            0,
            &[],
            RpcFragmentSizes::DEFAULT,
            RpcAuthenticationInfo {
                verifier_length: 0,
                ..authentication
            },
        ),
        Err(RpcPduError::EmptyAuthenticationVerifier)
    );
}

#[test]
fn authenticated_response_fragments_keep_verifiers_out_of_reassembly() {
    let authentication = RpcAuthenticationInfo {
        auth_type: 0x0a,
        auth_level: 5,
        auth_context_id: 1,
        verifier_length: 4,
    };
    let first = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_FIRST_FRAG,
        9,
        &[5, 0, 0, 0, 0, 0, 1, 2, 1, 2],
        authentication,
    )
    .expect("first response fragment")
    .finish(&[1, 2, 3, 4])
    .expect("reserved verifier length");
    let last = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_LAST_FRAG,
        9,
        &[3, 0, 0, 0, 0, 0, 0, 0, 3, 4, 5],
        authentication,
    )
    .expect("last response fragment")
    .finish(&[5, 6, 7, 8])
    .expect("reserved verifier length");

    let mut reassembler = RpcAuthenticatedResponseReassembler::new(5);
    let first = decode_rpc_authenticated_fragment(&first, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated first response")
        .verify_with(|segments, verifier| {
            assert_eq!(segments.body.len(), 16);
            assert_eq!(verifier, &[1, 2, 3, 4]);
            Ok::<(), ()>(())
        })
        .expect("caller accepts verifier");
    assert_eq!(reassembler.push(first), Ok(None));
    let mismatched = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        0,
        9,
        &[3, 0, 0, 0, 0, 0, 0, 0, 3, 4, 5],
        RpcAuthenticationInfo {
            auth_context_id: 2,
            ..authentication
        },
    )
    .expect("mismatched response fragment")
    .finish(&[5, 6, 7, 8])
    .expect("reserved verifier length");
    let mismatched = decode_rpc_authenticated_fragment(&mismatched, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated mismatched response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(
        reassembler.push(mismatched),
        Err(RpcPduError::ResponseFragmentAuthentication {
            expected_auth_type: 0x0a,
            expected_auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
            expected_auth_context_id: 1,
            actual_auth_type: 0x0a,
            actual_auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
            actual_auth_context_id: 2,
        })
    );
    let last = decode_rpc_authenticated_fragment(&last, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated last response")
        .verify_with(|_, verifier| {
            assert_eq!(verifier, &[5, 6, 7, 8]);
            Ok::<(), ()>(())
        })
        .expect("caller accepts verifier");
    assert_eq!(
        reassembler.push(last),
        Ok(Some(RpcReassembledResponse {
            call_id: 9,
            cancel_count: 0,
            reserved: 2,
            stub: vec![1, 2, 3, 4, 5],
        }))
    );
}

#[test]
fn authenticated_response_reassembly_resets_security_context_after_terminal_failure() {
    let authentication = RpcAuthenticationInfo {
        auth_type: 0x0a,
        auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
        auth_context_id: 1,
        verifier_length: 3,
    };
    let first = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_FIRST_FRAG,
        1,
        &[5, 0, 0, 0, 0, 0, 0, 0, 1, 2],
        authentication,
    )
    .expect("first response fragment")
    .finish(&[1, 2, 3])
    .expect("reserved verifier length");
    let invalid_last = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_LAST_FRAG,
        1,
        &[5, 0, 0, 0, 0, 0, 0, 0, 3],
        authentication,
    )
    .expect("invalid last response fragment")
    .finish(&[1, 2, 3])
    .expect("reserved verifier length");

    let mut reassembler = RpcAuthenticatedResponseReassembler::new(8);
    let first = decode_rpc_authenticated_fragment(&first, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated first response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(reassembler.push(first), Ok(None));
    let invalid_last = decode_rpc_authenticated_fragment(&invalid_last, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated invalid last response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(
        reassembler.push(invalid_last),
        Err(RpcPduError::InvalidAllocHint {
            alloc_hint: 5,
            stub_length: 3,
        })
    );

    let replacement = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_FIRST_FRAG | PFC_LAST_FRAG,
        2,
        &[1, 0, 0, 0, 0, 0, 0, 0, 9],
        RpcAuthenticationInfo {
            auth_context_id: 2,
            ..authentication
        },
    )
    .expect("replacement response")
    .finish(&[1, 2, 3])
    .expect("reserved verifier length");
    let replacement = decode_rpc_authenticated_fragment(&replacement, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated replacement response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(
        reassembler.push(replacement),
        Ok(Some(RpcReassembledResponse {
            call_id: 2,
            cancel_count: 0,
            reserved: 0,
            stub: vec![9],
        }))
    );
}

#[test]
fn authenticated_response_reassembly_resets_after_terminal_decode_failure() {
    let authentication = RpcAuthenticationInfo {
        auth_type: 0x0a,
        auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
        auth_context_id: 1,
        verifier_length: 3,
    };
    let first = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_FIRST_FRAG,
        1,
        &[1, 0, 0, 0, 0, 0, 0, 0, 1],
        authentication,
    )
    .expect("first response fragment")
    .finish(&[1, 2, 3])
    .expect("reserved verifier length");
    let invalid_last =
        prepare_rpc_authenticated_pdu(PTYPE_RESPONSE, PFC_LAST_FRAG, 1, &[1, 0, 0, 0, 0, 0, 0], authentication)
            .expect("invalid last response fragment")
            .finish(&[1, 2, 3])
            .expect("reserved verifier length");

    let mut reassembler = RpcAuthenticatedResponseReassembler::new(1);
    let first = decode_rpc_authenticated_fragment(&first, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated first response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(reassembler.push(first), Ok(None));
    let invalid_last = decode_rpc_authenticated_fragment(&invalid_last, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated invalid last response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(
        reassembler.push(invalid_last),
        Err(RpcPduError::Truncated { actual: 7, required: 8 })
    );

    let replacement = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_FIRST_FRAG | PFC_LAST_FRAG,
        2,
        &[1, 0, 0, 0, 0, 0, 0, 0, 2],
        RpcAuthenticationInfo {
            auth_context_id: 2,
            ..authentication
        },
    )
    .expect("replacement response")
    .finish(&[1, 2, 3])
    .expect("reserved verifier length");
    let replacement = decode_rpc_authenticated_fragment(&replacement, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated replacement response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(
        reassembler.push(replacement),
        Ok(Some(RpcReassembledResponse {
            call_id: 2,
            cancel_count: 0,
            reserved: 0,
            stub: vec![2],
        }))
    );
}

#[test]
fn authenticated_response_reassembly_recovers_after_terminal_authentication_mismatch() {
    let authentication = RpcAuthenticationInfo {
        auth_type: 0x0a,
        auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
        auth_context_id: 1,
        verifier_length: 1,
    };
    let first = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_FIRST_FRAG,
        1,
        &[2, 0, 0, 0, 0, 0, 0, 0, 1],
        authentication,
    )
    .expect("first response fragment")
    .finish(&[1])
    .expect("reserved verifier length");
    let mismatched_last = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_LAST_FRAG,
        1,
        &[1, 0, 0, 0, 0, 0, 0, 0, 2],
        RpcAuthenticationInfo {
            auth_context_id: 2,
            ..authentication
        },
    )
    .expect("mismatched response fragment")
    .finish(&[1])
    .expect("reserved verifier length");

    let mut reassembler = RpcAuthenticatedResponseReassembler::new(2);
    let first = decode_rpc_authenticated_fragment(&first, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated first response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(reassembler.push(first), Ok(None));
    let mismatched_last = decode_rpc_authenticated_fragment(&mismatched_last, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated mismatched response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(
        reassembler.push(mismatched_last),
        Err(RpcPduError::ResponseFragmentAuthentication {
            expected_auth_type: 0x0a,
            expected_auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
            expected_auth_context_id: 1,
            actual_auth_type: 0x0a,
            actual_auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
            actual_auth_context_id: 2,
        })
    );

    let replacement = prepare_rpc_authenticated_pdu(
        PTYPE_RESPONSE,
        PFC_FIRST_FRAG | PFC_LAST_FRAG,
        2,
        &[1, 0, 0, 0, 0, 0, 0, 0, 3],
        RpcAuthenticationInfo {
            auth_context_id: 2,
            ..authentication
        },
    )
    .expect("replacement response")
    .finish(&[1])
    .expect("reserved verifier length");
    let replacement = decode_rpc_authenticated_fragment(&replacement, DEFAULT_FRAGMENT_SIZE)
        .expect("authenticated replacement response")
        .verify_with(|_, _| Ok::<(), ()>(()))
        .expect("caller accepts verifier");
    assert_eq!(
        reassembler.push(replacement),
        Ok(Some(RpcReassembledResponse {
            call_id: 2,
            cancel_count: 0,
            reserved: 0,
            stub: vec![3],
        }))
    );
}

#[test]
fn authenticated_pdu_framing_rejects_malformed_trailers_and_verifiers() {
    let authentication = RpcAuthenticationInfo {
        auth_type: 0x0a,
        auth_level: 5,
        auth_context_id: 1,
        verifier_length: 3,
    };
    let prepared =
        prepare_rpc_authenticated_pdu(PTYPE_REQUEST, PFC_FIRST_FRAG | PFC_LAST_FRAG, 1, &[1], authentication)
            .expect("authenticated request");
    assert_eq!(
        prepared.clone().finish(&[1, 2]),
        Err(RpcPduError::AuthenticationVerifierLength { expected: 3, actual: 2 })
    );
    let valid = prepared.finish(&[1, 2, 3]).expect("reserved verifier length");
    assert_eq!(
        prepare_rpc_authenticated_pdu(
            PTYPE_REQUEST,
            PFC_FIRST_FRAG | PFC_LAST_FRAG,
            1,
            &[],
            RpcAuthenticationInfo {
                auth_level: RPC_AUTH_LEVEL_PACKET_INTEGRITY + 1,
                ..authentication
            },
        ),
        Err(RpcPduError::UnsupportedAuthenticationLevel {
            actual: RPC_AUTH_LEVEL_PACKET_INTEGRITY + 1,
        })
    );

    let mut oversized_trailer = valid.clone();
    oversized_trailer[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(
        decode_rpc_authenticated_fragment(&oversized_trailer, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::InvalidSecurityTrailer {
            fragment_length: 43,
            auth_length: u16::MAX,
        })
    );

    let mut unaligned_trailer = valid.clone();
    unaligned_trailer[8..10].copy_from_slice(&42u16.to_le_bytes());
    assert_eq!(
        decode_rpc_authenticated_fragment(&unaligned_trailer, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::UnalignedSecurityTrailer { offset: 31 })
    );

    let mut excessive_padding = valid.clone();
    excessive_padding[34] = 17;
    assert_eq!(
        decode_rpc_authenticated_fragment(&excessive_padding, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::InvalidAuthenticationPadding { actual: 17 })
    );
    let mut unsupported_level = valid.clone();
    unsupported_level[33] = RPC_AUTH_LEVEL_PACKET_INTEGRITY + 1;
    assert_eq!(
        decode_rpc_authenticated_fragment(&unsupported_level, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::UnsupportedAuthenticationLevel {
            actual: RPC_AUTH_LEVEL_PACKET_INTEGRITY + 1,
        })
    );
    assert_eq!(
        decode_rpc_authenticated_fragment(&valid, 42),
        Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length: 43,
            maximum: 42,
        })
    );
}

#[test]
fn bind_and_request_fragment_vectors() {
    let fault = [
        5,
        0,
        PTYPE_FAULT,
        PFC_FIRST_FRAG | PFC_LAST_FRAG,
        0x10,
        0,
        0,
        0,
        32,
        0,
        0,
        0,
        0x12,
        0x34,
        0x56,
        0x78,
        0,
        0,
        0,
        0,
        7,
        0,
        0,
        0,
        0xef,
        0xbe,
        0xad,
        0xde,
        0,
        0,
        0,
        0,
    ];
    assert_eq!(
        decode_rpc_fault(&fault, DEFAULT_FRAGMENT_SIZE),
        Err(RpcPduError::UnexpectedContextId { expected: 0, actual: 7 })
    );
    assert_eq!(
        decode_rpc_fault_for_context(&fault, DEFAULT_FRAGMENT_SIZE, 7).expect("valid fault"),
        RpcFault {
            call_id: 0x7856_3412,
            pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
            alloc_hint: 0,
            cancel_count: 0,
            reserved: 0,
            status: 0xdead_beef,
            reserved2: 0,
            stub: &[],
        }
    );

    let bind_nak = [
        5,
        0,
        PTYPE_BIND_NAK,
        PFC_FIRST_FRAG | PFC_LAST_FRAG,
        0x10,
        0,
        0,
        0,
        23,
        0,
        0,
        0,
        0x12,
        0x34,
        0x56,
        0x78,
        4,
        0,
        2,
        5,
        0,
        5,
        1,
    ];
    let nak = decode_rpc_bind_nak(&bind_nak, DEFAULT_FRAGMENT_SIZE).expect("valid bind rejection");
    assert_eq!(nak.call_id, 0x7856_3412);
    assert_eq!(nak.reason, 4);
    assert_eq!(
        nak.supported_versions,
        vec![
            ironrdp_mstsgu::rpc::RpcProtocolVersion::new(5, 0),
            ironrdp_mstsgu::rpc::RpcProtocolVersion::new(5, 1),
        ]
    );
    assert_eq!(nak.extended_error_signature, None);

    let requests = encode_rpc_request_fragments(
        0x7856_3412,
        7,
        3,
        &[1, 2, 3, 4, 5, 6, 7, 8, 9],
        RpcFragmentSizes::new(28, 28).expect("valid maxima"),
    )
    .expect("fragmented request");
    let common_header_size = u16::try_from(RPC_COMMON_HEADER_SIZE).expect("common header size fits");
    assert_eq!(
        encode_rpc_request_fragments(
            1,
            7,
            3,
            &[],
            RpcFragmentSizes::new(common_header_size, common_header_size).expect("valid common-header maximum"),
        ),
        Err(RpcPduError::FragmentTooSmall {
            maximum: common_header_size,
            required: RPC_COMMON_HEADER_SIZE + 8,
        })
    );
    assert_eq!(
        requests,
        vec![
            vec![
                5,
                0,
                PTYPE_REQUEST,
                PFC_FIRST_FRAG,
                0x10,
                0,
                0,
                0,
                28,
                0,
                0,
                0,
                0x12,
                0x34,
                0x56,
                0x78,
                9,
                0,
                0,
                0,
                7,
                0,
                3,
                0,
                1,
                2,
                3,
                4,
            ],
            vec![
                5,
                0,
                PTYPE_REQUEST,
                0,
                0x10,
                0,
                0,
                0,
                28,
                0,
                0,
                0,
                0x12,
                0x34,
                0x56,
                0x78,
                5,
                0,
                0,
                0,
                7,
                0,
                3,
                0,
                5,
                6,
                7,
                8,
            ],
            vec![
                5,
                0,
                PTYPE_REQUEST,
                PFC_LAST_FRAG,
                0x10,
                0,
                0,
                0,
                25,
                0,
                0,
                0,
                0x12,
                0x34,
                0x56,
                0x78,
                1,
                0,
                0,
                0,
                7,
                0,
                3,
                0,
                9,
            ],
        ]
    );
}

#[test]
fn ntlm_association_codecs_match_dce_rpc_wire_layouts() {
    let type1_token = [1, 2, 3, 4];
    let abstract_syntax = RpcSyntaxIdentifier::new(
        Uuid::from_u128(0x00112233_4455_6677_8899_aabbccddeeff),
        RpcSyntaxVersion::new(1, 3),
    );
    let transfer_syntax = RpcSyntaxIdentifier::new(
        Uuid::from_u128(0x8a885d04_1ceb_11c9_9fe8_08002b104860),
        RpcSyntaxVersion::new(2, 0),
    );
    let presentation_context = RpcPresentationContext {
        context_id: 7,
        abstract_syntax,
        transfer_syntaxes: &[transfer_syntax],
    };
    let bind = encode_rpc_bind_with_ntlm_auth(
        0x7856_3412,
        RpcFragmentSizes::DEFAULT,
        0,
        &[presentation_context],
        &type1_token,
    )
    .expect("valid authenticated bind");
    assert_eq!(
        bind,
        [
            [
                5,
                0,
                PTYPE_BIND,
                PFC_FIRST_FRAG | PFC_LAST_FRAG | PFC_SUPPORT_HEADER_SIGN,
                0x10,
                0,
                0,
                0,
            ]
            .as_slice(),
            &[92, 0, 4, 0, 0x12, 0x34, 0x56, 0x78],
            &[
                0xb8, 0x10, 0xb8, 0x10, 0, 0, 0, 0, 1, 0, 0, 0, 7, 0, 1, 0, 0x33, 0x22, 0x11, 0, 0x55, 0x44, 0x77,
                0x66, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 1, 0, 3, 0, 4, 0x5d, 0x88, 0x8a, 0xeb, 0x1c,
                0xc9, 0x11, 0x9f, 0xe8, 8, 0, 0x2b, 0x10, 0x48, 0x60, 2, 0, 0, 0,
            ],
            &[0; 8],
            &[10, 5, 8, 0, 1, 0, 0, 0],
            &type1_token,
        ]
        .concat()
    );

    let type3_token = [5, 6, 7, 8];
    let auth3 = encode_rpc_auth_3(0x7856_3412, RpcFragmentSizes::DEFAULT, &type3_token).expect("valid auth3");
    assert_eq!(
        auth3,
        [
            [
                5,
                0,
                PTYPE_RPC_AUTH_3,
                PFC_FIRST_FRAG | PFC_LAST_FRAG | PFC_SUPPORT_HEADER_SIGN,
                0x10,
                0,
                0,
                0,
            ]
            .as_slice(),
            &[44, 0, 4, 0, 0x12, 0x34, 0x56, 0x78],
            &[0; 16],
            &[10, 5, 12, 0, 1, 0, 0, 0],
            &type3_token,
        ]
        .concat()
    );
    assert_eq!(
        encode_rpc_auth_3(1, RpcFragmentSizes::DEFAULT, &[]),
        Err(RpcPduError::EmptyAuthenticationToken)
    );
}

#[test]
fn authenticated_bind_ack_extracts_the_type2_token_and_validates_its_trailer() {
    let type2_token = b"NTLMSSP\0\x02\0\0\0";
    let bind_ack = [
        [
            5,
            0,
            PTYPE_BIND_ACK,
            PFC_FIRST_FRAG | PFC_LAST_FRAG | PFC_SUPPORT_HEADER_SIGN,
            0x10,
            0,
            0,
            0,
        ]
        .as_slice(),
        &[84, 0, 12, 0, 0x12, 0x34, 0x56, 0x78],
        &[
            0x00, 0x20, 0x00, 0x30, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 4, 0x5d, 0x88, 0x8a, 0xeb, 0x1c,
            0xc9, 0x11, 0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60, 2, 0, 0, 0,
        ],
        &[0; 8],
        &[10, 5, 8, 0, 1, 0, 0, 0],
        type2_token.as_slice(),
    ]
    .concat();

    let ack = decode_rpc_bind_ack_with_ntlm_auth(&bind_ack, RpcFragmentSizes::DEFAULT)
        .expect("valid authenticated bind acknowledgement");
    assert_eq!(ack.token(), type2_token);
    assert_eq!(ack.bind_ack().call_id, 0x7856_3412);
    assert_eq!(
        ack.bind_ack().fragment_sizes,
        RpcFragmentSizes::new(0x10b8, 0x10b8).expect("valid negotiated maxima")
    );
    assert_eq!(ack.bind_ack().results.len(), 1);
    assert!(ack.supports_header_signing());

    let mut missing_header_sign = bind_ack.clone();
    missing_header_sign[3] &= !PFC_SUPPORT_HEADER_SIGN;
    let ack = decode_rpc_bind_ack_with_ntlm_auth(&missing_header_sign, RpcFragmentSizes::DEFAULT)
        .expect("valid bind acknowledgement without header-signing support");
    assert!(!ack.supports_header_signing());

    let mut wrong_auth_type = bind_ack.clone();
    wrong_auth_type[64] = 9;
    assert_eq!(
        decode_rpc_bind_ack_with_ntlm_auth(&wrong_auth_type, RpcFragmentSizes::DEFAULT),
        Err(RpcPduError::UnexpectedAuthenticationType {
            expected: 10,
            actual: 9,
        })
    );

    let mut wrong_auth_level = bind_ack.clone();
    wrong_auth_level[65] = 2;
    assert_eq!(
        decode_rpc_bind_ack_with_ntlm_auth(&wrong_auth_level, RpcFragmentSizes::DEFAULT),
        Err(RpcPduError::UnexpectedAuthenticationLevel { expected: 5, actual: 2 })
    );

    let mut nonzero_reserved = bind_ack.clone();
    nonzero_reserved[67] = 1;
    assert_eq!(
        decode_rpc_bind_ack_with_ntlm_auth(&nonzero_reserved, RpcFragmentSizes::DEFAULT),
        Err(RpcPduError::NonZeroAuthenticationReserved { actual: 1 })
    );

    let mut wrong_context = bind_ack.clone();
    wrong_context[68..72].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        decode_rpc_bind_ack_with_ntlm_auth(&wrong_context, RpcFragmentSizes::DEFAULT),
        Err(RpcPduError::UnexpectedAuthenticationContextId { expected: 1, actual: 0 })
    );

    let mut nonzero_padding = bind_ack.clone();
    nonzero_padding[63] = 1;
    assert_eq!(
        decode_rpc_bind_ack_with_ntlm_auth(&nonzero_padding, RpcFragmentSizes::DEFAULT),
        Err(RpcPduError::NonZeroAuthenticationPadding)
    );

    let mut oversized_padding = bind_ack.clone();
    oversized_padding[66] = 49;
    assert_eq!(
        decode_rpc_bind_ack_with_ntlm_auth(&oversized_padding, RpcFragmentSizes::DEFAULT),
        Err(RpcPduError::InvalidAuthenticationPadding { actual: 49 })
    );

    let mut invalid_trailer_bounds = bind_ack;
    invalid_trailer_bounds[10..12].copy_from_slice(&69u16.to_le_bytes());
    assert_eq!(
        decode_rpc_bind_ack_with_ntlm_auth(&invalid_trailer_bounds, RpcFragmentSizes::DEFAULT),
        Err(RpcPduError::InvalidSecurityTrailer {
            fragment_length: 84,
            auth_length: 69,
        })
    );
}

#[test]
fn rpc_ntlm_auth_requires_a_type1_type2_type3_sequence() {
    let mut auth = RpcNtlmAuth::new(r"CONTOSO\alice", "secret").expect("valid credentials");
    assert!(auth.continue_token(&[]).is_err());

    let type1_token = auth.initial_token().expect("offline Type-1 token");
    assert_eq!(&type1_token[..12], b"NTLMSSP\0\x01\0\0\0");
    assert!(!auth.is_complete());
    assert!(auth.initial_token().is_err());

    let type2_token = [
        b"NTLMSSP\0".as_slice(),
        &[2, 0, 0, 0],
        &[8, 0, 8, 0, 56, 0, 0, 0],
        &0xe288_82b7u32.to_le_bytes(),
        &[0x26, 0x6e, 0xcd, 0x75, 0xaa, 0x41, 0xe7, 0x6f],
        &[0; 8],
        &[64, 0, 64, 0, 64, 0, 0, 0],
        &[6, 1, 0xb0, 0x1d, 0, 0, 0, 0x0f],
        &[0x57, 0, 0x49, 0, 0x4e, 0, 0x37, 0],
        &[
            2, 0, 8, 0, 0x57, 0, 0x49, 0, 0x4e, 0, 0x37, 0, 1, 0, 8, 0, 0x57, 0, 0x49, 0, 0x4e, 0, 0x37, 0, 4, 0, 8, 0,
            0x77, 0, 0x69, 0, 0x6e, 0, 0x37, 0, 3, 0, 8, 0, 0x77, 0, 0x69, 0, 0x6e, 0, 0x37, 0, 7, 0, 8, 0, 0xa9, 0x8d,
            0x9b, 0x1a, 0x6c, 0xb0, 0xcb, 1, 0, 0, 0, 0,
        ],
    ]
    .concat();
    let type3_token = auth.continue_token(&type2_token).expect("valid Type-2 token");
    assert_eq!(&type3_token[..12], b"NTLMSSP\0\x03\0\0\0");
    assert!(auth.is_complete());
    assert!(auth.continue_token(&type2_token).is_err());
}
