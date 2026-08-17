use ironrdp_core::{DecodeResult, Encode as _, decode, encode_vec};
use ironrdp_rdpeudp::pdu::*;
// ════════════════════════════════════════════════════════════════
// V1Datagram tests
// ════════════════════════════════════════════════════════════════

/// Client SYN: header + SYNDATA + SYNDATAEX.
#[test]
fn v1_syn_datagram_roundtrip() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 0x1234_5678,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            udp_ver: UdpVersion::V2,
            cookie_hash: None,
        }),
    };

    // header(8) + syndata(8) + syndataex(4) = 20 bytes
    assert_eq!(datagram.size(), 20);

    let encoded = encode_vec(&datagram).expect("encode");
    assert_eq!(encoded.len(), 20);

    let decoded: V1Datagram = decode(&encoded).expect("decode");
    assert_eq!(decoded, datagram);
}

/// Server SYN+ACK: header + SYNDATA + SYNDATAEX.
///
/// No ACK vector, despite the ACK flag: see [MS-RDPEUDP] 3.1.5.1.3.
#[test]
fn v1_syn_ack_datagram_roundtrip() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 100,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::ACK | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 0xAABB_CCDD,
            upstream_mtu: 1200,
            downstream_mtu: 1200,
        }),
        correlation_id: None,
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            udp_ver: UdpVersion::V2,
            cookie_hash: None,
        }),
    };

    // header(8) + syndata(8) + syndataex(4) = 20.
    assert_eq!(datagram.size(), 20);

    let encoded = encode_vec(&datagram).expect("encode");
    let decoded: V1Datagram = decode(&encoded).expect("decode");
    assert_eq!(decoded, datagram);
    assert!(decoded.header.flags.contains(V1Flags::ACK));
}

/// The SYN+ACK capture from [MS-RDPEUDP] 4.1.2, decoded whole.
///
/// The ACK flag is set and the SYNDATA payload starts immediately after the
/// 8-byte header, so a decoder that reads an ACK vector here consumes the
/// first half of SYNDATA and desynchronises for the rest of the datagram.
#[test]
fn v1_decode_the_spec_syn_ack_capture() {
    // Trailing zeroes are the start of the pad to uUpStreamMtu.
    const CAPTURE: [u8; 19] = [
        0x00, 0x00, 0x00, 0x42, 0x04, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x42, 0x04, 0xD0, 0x04, 0xD0, 0x00, 0x00,
        0x00,
    ];

    let datagram: V1Datagram = decode(&CAPTURE).expect("decode");

    assert_eq!(datagram.header.sn_source_ack, 0x42);
    assert_eq!(datagram.header.receive_window_size, 1024);
    assert_eq!(datagram.header.flags, V1Flags::SYN | V1Flags::ACK);
    assert!(datagram.ack_vector.is_none());

    let syn_data = datagram.syn_data.expect("SYNDATA");
    assert_eq!(syn_data.initial_sequence_number, 0x42);
    assert_eq!(syn_data.upstream_mtu, 1232);
    assert_eq!(syn_data.downstream_mtu, 1232);

    assert!(datagram.syn_data_ex.is_none());
}

/// The ACK capture from [MS-RDPEUDP] 4.2.3, whose vector we do decode.
///
/// The counterpart to the SYN+ACK above: without SYN, the ACK flag means what
/// 2.2.2.1 says it means. The DATA payload is left off, since v1 data transfer
/// is out of scope for this crate.
#[test]
fn v1_decode_the_spec_ack_capture() {
    const CAPTURE: [u8; 16] = [
        // FEC header: snSourceAck, uReceiveWindowSize 1024, uFlags 0x0104.
        0xD6, 0xCF, 0x0A, 0xB8, 0x04, 0x00, 0x01, 0x04,
        // ACK vector: one element, 4 datagrams received, then a pad byte.
        0x00, 0x01, 0x04, 0x00, //
        // AckOfAcks.
        0xD6, 0xCF, 0x0A, 0xB8,
    ];

    let datagram: V1Datagram = decode(&CAPTURE).expect("decode");

    assert_eq!(datagram.header.sn_source_ack, 0xD6CF_0AB8);
    assert_eq!(datagram.header.receive_window_size, 1024);

    let ack_vector = datagram.ack_vector.as_ref().expect("ACK vector");
    assert_eq!(
        ack_vector.elements,
        vec![V1AckVectorElement {
            state: VectorElementState::DatagramReceived,
            length: 4,
        }]
    );

    assert_eq!(
        datagram.ack_of_acks.as_ref().expect("AckOfAcks").reset_seq_num,
        0xD6CF_0AB8
    );

    assert_eq!(encode_vec(&datagram).expect("encode"), CAPTURE);
}

/// A SYN datagram carrying an ACK vector is rejected rather than written.
#[test]
fn v1_encode_rejects_ack_vector_on_a_syn() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 100,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::ACK,
        },
        ack_vector: Some(V1AckVectorHeader {
            elements: vec![V1AckVectorElement {
                state: VectorElementState::DatagramReceived,
                length: 1,
            }],
        }),
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 1,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: None,
    };

    encode_vec(&datagram).expect_err("a SYN datagram cannot carry an ACK vector");
}

/// Client final ACK: header + ack_vector + ack_of_acks.
#[test]
fn v1_ack_datagram_roundtrip() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 200,
            receive_window_size: 64,
            flags: V1Flags::ACK | V1Flags::ACK_OF_ACKS,
        },
        ack_vector: Some(V1AckVectorHeader {
            elements: vec![
                V1AckVectorElement {
                    state: VectorElementState::DatagramReceived,
                    length: 5,
                },
                V1AckVectorElement {
                    state: VectorElementState::DatagramNotYetReceived,
                    length: 2,
                },
            ],
        }),
        ack_of_acks: Some(V1AckOfAcksHeader { reset_seq_num: 150 }),
        syn_data: None,
        correlation_id: None,
        syn_data_ex: None,
    };

    // header(8) + ack_vector(2+2=4) + ack_of_acks(4) = 16
    assert_eq!(datagram.size(), 16);

    let encoded = encode_vec(&datagram).expect("encode");
    let decoded: V1Datagram = decode(&encoded).expect("decode");
    assert_eq!(decoded, datagram);
}

/// SYN with correlation ID.
#[test]
fn v1_syn_with_correlation_id_roundtrip() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::SYNEX | V1Flags::CORRELATION_ID,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 42,
            upstream_mtu: 1232,
            downstream_mtu: 1132,
        }),
        correlation_id: Some(CorrelationIdPayload {
            correlation_id: [
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
            ],
        }),
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            udp_ver: UdpVersion::V2,
            cookie_hash: None,
        }),
    };

    // header(8) + syndata(8) + correlation(16 id + 16 reserved = 32) + syndataex(4) = 52.
    // [MS-RDPEUDP] 2.2.2.8 makes RDPUDP_CORRELATION_ID_PAYLOAD 32 bytes.
    assert_eq!(datagram.size(), 52);

    let encoded = encode_vec(&datagram).expect("encode");
    let decoded: V1Datagram = decode(&encoded).expect("decode");
    assert_eq!(decoded, datagram);
}

/// V3 SYN with 32-byte cookie hash.
#[test]
fn v1_syn_v3_with_cookie_roundtrip() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 0xDEAD_BEEF,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            udp_ver: UdpVersion::V3,
            cookie_hash: Some([0xAA; 32]),
        }),
    };

    // header(8) + syndata(8) + syndataex(4+32=36) = 52
    assert_eq!(datagram.size(), 52);

    let encoded = encode_vec(&datagram).expect("encode");
    let decoded: V1Datagram = decode(&encoded).expect("decode");
    assert_eq!(decoded, datagram);
}

/// A v3 SYN+ACK padded to the MTU must not have its trailing padding read as
/// a cookieHash. [MS-RDPEUDP] 2.2.2.9 ties cookieHash presence to direction
/// (client-to-server SYN only); a length-based heuristic misreads padding
/// bytes on a SYN+ACK as a fabricated 32-byte hash.
#[test]
fn v1_syn_ack_v3_does_not_read_padding_as_cookie_hash() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 100,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::ACK | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 0xAABB_CCDD,
            upstream_mtu: 1200,
            downstream_mtu: 1200,
        }),
        correlation_id: None,
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            udp_ver: UdpVersion::V3,
            cookie_hash: None,
        }),
    };

    let mut encoded = encode_vec(&datagram).expect("encode");
    // Simulate zero-padding out to the negotiated MTU (3.1.5.1.3): this is
    // exactly the trailing data a remaining-length heuristic would misread.
    encoded.resize(1200, 0);

    let decoded: V1Datagram = decode(&encoded).expect("decode");
    assert_eq!(
        decoded.syn_data_ex.expect("syn_data_ex present").cookie_hash,
        None,
        "padding must not be read as a cookieHash on a SYN+ACK"
    );
}

/// Verify flags are auto-computed from populated fields.
#[test]
fn v1_flags_auto_computed_on_encode() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            // Caller set neither ACK nor ACK_OF_ACKS, but the payloads
            // those flags gate are populated.
            flags: V1Flags::empty(),
        },
        ack_vector: Some(V1AckVectorHeader { elements: vec![] }),
        ack_of_acks: Some(V1AckOfAcksHeader { reset_seq_num: 7 }),
        syn_data: None,
        correlation_id: None,
        // syn_data_ex is None, so SYNEX should NOT be in flags
        syn_data_ex: None,
    };

    let encoded = encode_vec(&datagram).expect("encode");
    let decoded: V1Datagram = decode(&encoded).expect("decode");

    // ACK should be auto-added (ack_vector is Some)
    assert!(decoded.header.flags.contains(V1Flags::ACK));
    // ACK_OF_ACKS should be auto-added (ack_of_acks is Some)
    assert!(decoded.header.flags.contains(V1Flags::ACK_OF_ACKS));
    // SYNEX should NOT be set (syn_data_ex is None)
    assert!(!decoded.header.flags.contains(V1Flags::SYNEX));
}

/// The ACK flag survives on a SYN+ACK, where no payload implies it.
#[test]
fn v1_ack_flag_preserved_on_a_syn() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 100,
            receive_window_size: 64,
            flags: V1Flags::ACK,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 1,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: None,
    };

    let encoded = encode_vec(&datagram).expect("encode");
    let decoded: V1Datagram = decode(&encoded).expect("decode");

    assert_eq!(decoded.header.flags, V1Flags::SYN | V1Flags::ACK);
    assert!(decoded.ack_vector.is_none());
}

/// A plain SYN does not acquire the ACK flag.
#[test]
fn v1_ack_flag_absent_on_a_bare_syn() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::empty(),
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 1,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: None,
    };

    let encoded = encode_vec(&datagram).expect("encode");
    let decoded: V1Datagram = decode(&encoded).expect("decode");

    assert_eq!(decoded.header.flags, V1Flags::SYN);
}

/// Standalone flags (CN, CWR, ACKDELAYED) are preserved on encode.
#[test]
fn v1_standalone_flags_preserved() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 100,
            receive_window_size: 64,
            flags: V1Flags::ACK | V1Flags::CN | V1Flags::ACKDELAYED,
        },
        ack_vector: Some(V1AckVectorHeader {
            elements: vec![V1AckVectorElement {
                state: VectorElementState::DatagramReceived,
                length: 10,
            }],
        }),
        ack_of_acks: None,
        syn_data: None,
        correlation_id: None,
        syn_data_ex: None,
    };

    let encoded = encode_vec(&datagram).expect("encode");
    let decoded: V1Datagram = decode(&encoded).expect("decode");

    assert!(decoded.header.flags.contains(V1Flags::CN));
    assert!(decoded.header.flags.contains(V1Flags::ACKDELAYED));
    assert!(decoded.header.flags.contains(V1Flags::ACK));
}

/// Reject datagrams with DATA flag (not supported in handshake).
#[test]
fn v1_decode_rejects_data_flag() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0,
            receive_window_size: 64,
            flags: V1Flags::empty(),
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: None,
        correlation_id: None,
        syn_data_ex: None,
    };
    let mut encoded = encode_vec(&datagram).expect("encode");

    // Manually set the DATA flag in the wire bytes. Flags are at offset 6..8
    // in FecHeader (after snSourceAck(4) + windowSize(2)), network byte order
    // (big-endian) per the header codec's write_u16_be/read_u16_be.
    let flags_raw = u16::from_be_bytes([encoded[6], encoded[7]]);
    let modified = flags_raw | V1Flags::DATA.bits();
    let [high, low] = modified.to_be_bytes();
    encoded[6] = high;
    encoded[7] = low;

    let result: DecodeResult<V1Datagram> = decode(&encoded);
    assert!(result.is_err());
}

/// Reject datagrams with FEC flag (not supported in handshake).
#[test]
fn v1_decode_rejects_fec_flag() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0,
            receive_window_size: 64,
            flags: V1Flags::empty(),
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: None,
        correlation_id: None,
        syn_data_ex: None,
    };
    let mut encoded = encode_vec(&datagram).expect("encode");

    let flags_raw = u16::from_be_bytes([encoded[6], encoded[7]]);
    let modified = flags_raw | V1Flags::FEC.bits();
    let [high, low] = modified.to_be_bytes();
    encoded[6] = high;
    encoded[7] = low;

    let result: DecodeResult<V1Datagram> = decode(&encoded);
    assert!(result.is_err());
}

/// Minimal datagram: just a header with no payloads.
#[test]
fn v1_empty_datagram_roundtrip() {
    let datagram = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0,
            receive_window_size: 32,
            flags: V1Flags::empty(),
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: None,
        correlation_id: None,
        syn_data_ex: None,
    };

    assert_eq!(datagram.size(), 8); // header only

    let encoded = encode_vec(&datagram).expect("encode");
    let decoded: V1Datagram = decode(&encoded).expect("decode");
    assert_eq!(decoded, datagram);
}

/// Insufficient bytes for header.
#[test]
fn v1_insufficient_bytes() {
    let bytes = [0x00, 0x00, 0x00]; // need 8 for header
    let result: DecodeResult<V1Datagram> = decode(&bytes);
    assert!(result.is_err());
}
