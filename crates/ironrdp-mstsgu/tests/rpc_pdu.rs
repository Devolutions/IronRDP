#![allow(unused_crate_dependencies)]

use ironrdp_mstsgu::rpc::{
    DEFAULT_FRAGMENT_SIZE, MAX_PENDING_RPC_FRAGMENTS, PFC_FIRST_FRAG, PFC_LAST_FRAG, PTYPE_FAULT, PTYPE_RESPONSE,
    RPC_COMMON_HEADER_SIZE, RPC_DREP_LITTLE_ENDIAN, RPC_VERSION, RPC_VERSION_MINOR, RpcCommonHeader, RpcFault,
    RpcFragmentSizes, RpcPduError, RpcPduStream, RpcReassembledResponse, RpcResponse, RpcResponseReassembler,
    RpcSyntaxVersion, decode_rpc_fault, decode_rpc_response, decode_rpc_response_fragment, encode_rpc_fault,
    encode_rpc_response, encode_rpc_response_fragment,
};

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
        alloc_hint: 2,
        cancel_count: 0,
        reserved: 0,
        status: 0xdead_beef,
        reserved2: 1,
        stub: &[0x12, 0x34],
    };
    let encoded = encode_rpc_fault(fault).expect("valid fault");
    assert_eq!(encoded[2], PTYPE_FAULT);
    assert_eq!(
        decode_rpc_fault(&encoded, DEFAULT_FRAGMENT_SIZE).expect("valid fault"),
        fault
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
            cancel_count: 1,
            reserved: 2,
            stub: vec![1, 2, 3, 4, 5],
        }))
    );
}

#[test]
fn response_reassembler_rejects_call_id_mismatch_and_invalid_alloc_hint() {
    let mut reassembler = RpcResponseReassembler::new(8);
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

    let mut reassembler = RpcResponseReassembler::new(8);
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
