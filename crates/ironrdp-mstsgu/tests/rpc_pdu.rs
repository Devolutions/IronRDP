#![allow(unused_crate_dependencies)]

use ironrdp_mstsgu::rpc::{
    DEFAULT_FRAGMENT_SIZE, MAX_PENDING_RPC_FRAGMENTS, PFC_FIRST_FRAG, PFC_LAST_FRAG, PTYPE_BIND, PTYPE_BIND_ACK,
    PTYPE_BIND_NAK, PTYPE_FAULT, PTYPE_REQUEST, PTYPE_RESPONSE, RPC_COMMON_HEADER_SIZE, RPC_DREP_LITTLE_ENDIAN,
    RPC_VERSION, RPC_VERSION_MINOR, RpcCommonHeader, RpcFault, RpcFragmentSizes, RpcPduError, RpcPduStream,
    RpcPresentationContext, RpcReassembledResponse, RpcResponse, RpcResponseReassembler, RpcSyntaxIdentifier,
    RpcSyntaxVersion, decode_rpc_bind_ack, decode_rpc_bind_nak, decode_rpc_fault, decode_rpc_fault_for_context,
    decode_rpc_response, decode_rpc_response_for_context, decode_rpc_response_fragment, encode_rpc_bind,
    encode_rpc_fault, encode_rpc_request_fragments, encode_rpc_response, encode_rpc_response_fragment,
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
    let ack = decode_rpc_bind_ack(&bind_ack, RpcFragmentSizes::DEFAULT).expect("valid bind acknowledgement");
    assert_eq!(ack.call_id, 0x7856_3412);
    assert_eq!(ack.fragment_sizes, RpcFragmentSizes::DEFAULT);
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
        Err(RpcPduError::UnexpectedContextId { actual: 7 })
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
