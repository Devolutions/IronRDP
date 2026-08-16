use ironrdp_core::{DecodeResult, Encode as _, decode, encode_vec};
use ironrdp_rdpeudp::pdu::*;
// ════════════════════════════════════════════════════════════════
// V2Packet tests
// ════════════════════════════════════════════════════════════════

/// ACK-only packet.
#[test]
fn v2_ack_only_roundtrip() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::ACK,
            log_window_size: 6,
        },
        ack: Some(AckPayload {
            seq_num: 0x0042,
            received_ts: 0x00_123456,
            send_ack_time_gap: 5,
            delay_ack_time_scale: 0,
            delay_ack_time_additions: Vec::new(),
        }),
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: None,
        ack_vector: None,
        data_body: None,
    };

    // header(2) + ack(7) = 9
    assert_eq!(packet.size(), 9);

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");
    assert_eq!(decoded, packet);
}

/// DATA-only packet.
#[test]
fn v2_data_only_roundtrip() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::DATA,
            log_window_size: 8,
        },
        ack: None,
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: Some(DataHeader { data_seq_num: 0x0100 }),
        ack_vector: None,
        data_body: Some(DataBody {
            channel_seq_num: 0x0001,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF],
        }),
    };

    // header(2) + data_header(2) + data_body(2+4=6) = 10
    assert_eq!(packet.size(), 10);

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");
    assert_eq!(decoded, packet);
}

/// ACK + DATA combined.
#[test]
fn v2_ack_with_data_roundtrip() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::ACK | V2Flags::DATA,
            log_window_size: 6,
        },
        ack: Some(AckPayload {
            seq_num: 50,
            received_ts: 0x00_AABBCC,
            send_ack_time_gap: 10,
            delay_ack_time_scale: 3,
            delay_ack_time_additions: vec![15, 20],
        }),
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: Some(DataHeader { data_seq_num: 0x1234 }),
        ack_vector: None,
        data_body: Some(DataBody {
            channel_seq_num: 100,
            data: vec![0x01, 0x02, 0x03],
        }),
    };

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");
    assert_eq!(decoded, packet);
}

/// ACKVEC packet (mutually exclusive with ACK).
#[test]
fn v2_ackvec_roundtrip() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::ACKVEC,
            log_window_size: 10,
        },
        ack: None,
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: None,
        ack_vector: Some(AckVectorPayload {
            base_seq_num: 0x0100,
            timestamp: Some(0x00_AABBCC),
            send_ack_time_gap_ms: Some(25),
            entries: vec![
                AckVectorEntry::RunLength {
                    received: true,
                    length: 20,
                },
                AckVectorEntry::StateMap { bitmap: 0b0110_1010 },
            ],
        }),
        data_body: None,
    };

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");
    assert_eq!(decoded, packet);
}

/// All control payloads at once: OverheadSize + DelayAckInfo + AOA.
#[test]
fn v2_all_control_payloads_roundtrip() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::ACK | V2Flags::OVERHEADSIZE | V2Flags::DELAYACKINFO | V2Flags::AOA,
            log_window_size: 6,
        },
        ack: Some(AckPayload {
            seq_num: 10,
            received_ts: 500_000,
            send_ack_time_gap: 3,
            delay_ack_time_scale: 0,
            delay_ack_time_additions: Vec::new(),
        }),
        overhead_size: Some(OverheadSizePayload { overhead_size: 28 }),
        delay_ack_info: Some(DelayAckInfoPayload {
            max_delayed_acks: 8,
            delayed_ack_timeout_ms: 150,
        }),
        ack_of_acks: Some(AckOfAcksPayload { ack_of_acks_seq_num: 5 }),
        data_header: None,
        ack_vector: None,
        data_body: None,
    };

    // header(2) + ack(7) + overhead(1) + delay_ack_info(3) + aoa(2) = 15
    assert_eq!(packet.size(), 15);

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");
    assert_eq!(decoded, packet);
}

/// Full packet with ACK + OverheadSize + DelayAckInfo + AOA + DATA.
#[test]
fn v2_full_packet_roundtrip() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::ACK | V2Flags::OVERHEADSIZE | V2Flags::DELAYACKINFO | V2Flags::AOA | V2Flags::DATA,
            log_window_size: 8,
        },
        ack: Some(AckPayload {
            seq_num: 0x1000,
            received_ts: 0x00_FFFFFF,
            send_ack_time_gap: 50,
            delay_ack_time_scale: 2,
            delay_ack_time_additions: vec![10, 20, 30],
        }),
        overhead_size: Some(OverheadSizePayload { overhead_size: 40 }),
        delay_ack_info: Some(DelayAckInfoPayload {
            max_delayed_acks: 15,
            delayed_ack_timeout_ms: 500,
        }),
        ack_of_acks: Some(AckOfAcksPayload {
            ack_of_acks_seq_num: 0x0FF0,
        }),
        data_header: Some(DataHeader { data_seq_num: 0x2000 }),
        ack_vector: None,
        data_body: Some(DataBody {
            channel_seq_num: 0x1000,
            data: vec![0x55; 100],
        }),
    };

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");
    assert_eq!(decoded, packet);
}

/// Verify flags are auto-computed from populated fields.
#[test]
fn v2_flags_auto_computed_on_encode() {
    let packet = V2Packet {
        header: V2Header {
            // Caller only set DATA, but ack is also populated
            flags: V2Flags::DATA,
            log_window_size: 6,
        },
        ack: Some(AckPayload {
            seq_num: 1,
            received_ts: 0,
            send_ack_time_gap: 0,
            delay_ack_time_scale: 0,
            delay_ack_time_additions: Vec::new(),
        }),
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: Some(DataHeader { data_seq_num: 1 }),
        ack_vector: None,
        data_body: Some(DataBody {
            channel_seq_num: 1,
            data: vec![0x42],
        }),
    };

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");

    // ACK should be auto-added
    assert!(decoded.header.flags.contains(V2Flags::ACK));
    // DATA should be present
    assert!(decoded.header.flags.contains(V2Flags::DATA));
    // ACKVEC should NOT be set
    assert!(!decoded.header.flags.contains(V2Flags::ACKVEC));
}

/// The v2 header has no standalone flags, so a bit the caller sets without the
/// payload it announces does not survive the round trip.
///
/// Every flag [MS-RDPEUDP2] 2.2.1.1 defines announces a payload. There is
/// nothing like the v1 header's CN, CWR or SYNLOSSY, which stand on their own
/// and have to be carried across.
#[test]
fn v2_has_no_standalone_flags() {
    let packet = V2Packet {
        header: V2Header {
            // ACKVEC announces a payload this packet does not carry.
            flags: V2Flags::ACK | V2Flags::ACKVEC,
            log_window_size: 6,
        },
        ack: Some(AckPayload {
            seq_num: 1,
            received_ts: 0,
            send_ack_time_gap: 0,
            delay_ack_time_scale: 0,
            delay_ack_time_additions: Vec::new(),
        }),
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: None,
        ack_vector: None,
        data_body: None,
    };

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");

    assert_eq!(
        decoded.header.flags,
        V2Flags::ACK,
        "flags should describe exactly the payloads present"
    );
}

/// Encode rejects ACK + ACKVEC both set.
#[test]
fn v2_ack_and_ackvec_mutual_exclusion_encode() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::empty(),
            log_window_size: 6,
        },
        ack: Some(AckPayload {
            seq_num: 1,
            received_ts: 0,
            send_ack_time_gap: 0,
            delay_ack_time_scale: 0,
            delay_ack_time_additions: Vec::new(),
        }),
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: None,
        ack_vector: Some(AckVectorPayload {
            base_seq_num: 0,
            timestamp: None,
            send_ack_time_gap_ms: None,
            entries: Vec::new(),
        }),
        data_body: None,
    };

    let result = encode_vec(&packet);
    assert!(result.is_err());
}

/// Encode rejects data_header without data_body.
#[test]
fn v2_data_header_without_body_rejected() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::DATA,
            log_window_size: 6,
        },
        ack: None,
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: Some(DataHeader { data_seq_num: 1 }),
        ack_vector: None,
        data_body: None, // Missing!
    };

    let result = encode_vec(&packet);
    assert!(result.is_err());
}

/// Encode rejects data_body without data_header.
#[test]
fn v2_data_body_without_header_rejected() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::DATA,
            log_window_size: 6,
        },
        ack: None,
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: None, // Missing!
        ack_vector: None,
        data_body: Some(DataBody {
            channel_seq_num: 1,
            data: vec![0x01],
        }),
    };

    let result = encode_vec(&packet);
    assert!(result.is_err());
}

/// Empty packet: just header, no payloads.
#[test]
fn v2_empty_packet_roundtrip() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::empty(),
            log_window_size: 6,
        },
        ack: None,
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: None,
        ack_vector: None,
        data_body: None,
    };

    assert_eq!(packet.size(), 2); // header only

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");
    assert_eq!(decoded, packet);
}

/// Insufficient bytes for V2 header.
#[test]
fn v2_insufficient_bytes() {
    let bytes = [0x00]; // need 2 for header
    let result: DecodeResult<V2Packet> = decode(&bytes);
    assert!(result.is_err());
}

/// DataBody correctly consumes all remaining bytes.
#[test]
fn v2_data_body_consumes_remaining() {
    // Build a packet with ACK + DATA, then verify data_body gets all remaining
    let payload_data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::ACK | V2Flags::DATA,
            log_window_size: 6,
        },
        ack: Some(AckPayload {
            seq_num: 1,
            received_ts: 100,
            send_ack_time_gap: 0,
            delay_ack_time_scale: 0,
            delay_ack_time_additions: Vec::new(),
        }),
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: Some(DataHeader { data_seq_num: 1 }),
        ack_vector: None,
        data_body: Some(DataBody {
            channel_seq_num: 1,
            data: payload_data.clone(),
        }),
    };

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");

    let body = decoded.data_body.expect("data_body should be present");
    assert_eq!(body.data, payload_data);
}

/// ACKVEC + DATA combined (no ACK).
#[test]
fn v2_ackvec_with_data_roundtrip() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::ACKVEC | V2Flags::DATA,
            log_window_size: 6,
        },
        ack: None,
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: Some(DataHeader { data_seq_num: 0x0042 }),
        ack_vector: Some(AckVectorPayload {
            base_seq_num: 0x0100,
            timestamp: None,
            send_ack_time_gap_ms: None,
            entries: vec![AckVectorEntry::RunLength {
                received: true,
                length: 10,
            }],
        }),
        data_body: Some(DataBody {
            channel_seq_num: 0x0042,
            data: vec![0xAA, 0xBB, 0xCC],
        }),
    };

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");
    assert_eq!(decoded, packet);
}

/// The v2 header defines exactly six flags.
///
/// [MS-RDPEUDP2] 2.2.1.1 lists ACK, DATA, ACKVEC, AOA, OVERHEADSIZE and
/// DELAYACKINFO, and nothing else. In particular there is no DUMMY flag: a
/// dummy packet is marked by Packet_Type_Index 8 in the PacketPrefixByte
/// (3.1.1.1.5), one layer down.
#[test]
fn v2_header_flags_match_the_spec_table() {
    assert_eq!(V2Flags::ACK.bits(), 0x001);
    assert_eq!(V2Flags::DATA.bits(), 0x004);
    assert_eq!(V2Flags::ACKVEC.bits(), 0x008);
    assert_eq!(V2Flags::AOA.bits(), 0x010);
    assert_eq!(V2Flags::OVERHEADSIZE.bits(), 0x040);
    assert_eq!(V2Flags::DELAYACKINFO.bits(), 0x100);
}

/// Near-MTU data packet.
#[test]
fn v2_large_data_packet_roundtrip() {
    let packet = V2Packet {
        header: V2Header {
            flags: V2Flags::DATA,
            log_window_size: 10,
        },
        ack: None,
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: Some(DataHeader { data_seq_num: 1000 }),
        ack_vector: None,
        data_body: Some(DataBody {
            channel_seq_num: 500,
            data: vec![0xCC; 1200],
        }),
    };

    let encoded = encode_vec(&packet).expect("encode");
    let decoded: V2Packet = decode(&encoded).expect("decode");
    assert_eq!(decoded, packet);
}
