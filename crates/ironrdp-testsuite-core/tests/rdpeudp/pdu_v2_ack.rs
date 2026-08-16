use ironrdp_core::{DecodeResult, Encode as _, decode, encode_vec};
use ironrdp_rdpeudp::pdu::*;
// ── AckPayload tests ──

#[test]
fn ack_payload_no_delayed() {
    let ack = AckPayload {
        seq_num: 0x1234,
        received_ts: 0x00AB_CDEF,
        send_ack_time_gap: 5,
        delay_ack_time_scale: 0,
        delay_ack_time_additions: Vec::new(),
    };
    let encoded = encode_vec(&ack).expect("encode");
    assert_eq!(ack.size(), 7);
    assert_eq!(encoded.len(), 7);

    let decoded: AckPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded, ack);
}

#[test]
fn ack_payload_with_delayed() {
    let ack = AckPayload {
        seq_num: 0x0042,
        received_ts: 0x00_123456,
        send_ack_time_gap: 10,
        delay_ack_time_scale: 2,
        delay_ack_time_additions: vec![15, 20, 25],
    };
    let encoded = encode_vec(&ack).expect("encode");
    assert_eq!(ack.size(), 10); // 7 + 3
    assert_eq!(encoded.len(), 10);

    let decoded: AckPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded, ack);
}

#[test]
fn ack_payload_timestamp_24bit() {
    let ack = AckPayload {
        seq_num: 0,
        received_ts: 0x00FF_FFFF, // max 24-bit value
        send_ack_time_gap: 0,
        delay_ack_time_scale: 0,
        delay_ack_time_additions: Vec::new(),
    };
    let encoded = encode_vec(&ack).expect("encode");
    let decoded: AckPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded.received_ts, 0x00FF_FFFF);
}

#[test]
fn ack_payload_timestamp_masked() {
    // Setting bits above 24 should be masked on encode
    let ack = AckPayload {
        seq_num: 0,
        received_ts: 0xFFFF_FFFF, // bits above 24 set
        send_ack_time_gap: 0,
        delay_ack_time_scale: 0,
        delay_ack_time_additions: Vec::new(),
    };
    let encoded = encode_vec(&ack).expect("encode");
    let decoded: AckPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded.received_ts, 0x00FF_FFFF);
}

#[test]
fn ack_payload_nibble_packing() {
    // [MS-RDPEUDP2] 2.2.1.2.1: numDelayedAcks is the low nibble and
    // delayAckTimeScale the high nibble of byte 6.
    let ack = AckPayload {
        seq_num: 0,
        received_ts: 0,
        send_ack_time_gap: 0,
        delay_ack_time_scale: 8, // arbitrary 4-bit value
        delay_ack_time_additions: vec![0; 15],
    };
    let encoded = encode_vec(&ack).expect("encode");
    // (scale 8 << 4) | count 15 = 0x8F
    assert_eq!(encoded[6], 0x8F);

    let decoded: AckPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded.delay_ack_time_additions.len(), 15);
    assert_eq!(decoded.delay_ack_time_scale, 8);
}

#[test]
fn ack_payload_insufficient_bytes() {
    let bytes = [0x00, 0x00, 0x00]; // only 3 bytes, need 7
    let result: DecodeResult<AckPayload> = decode(&bytes);
    assert!(result.is_err());
}

#[test]
fn ack_payload_insufficient_additions() {
    // Claims 5 delayed acks but no addition bytes follow
    let bytes = [
        0x00, 0x00, // seq_num
        0x00, 0x00, 0x00, // received_ts
        0x00, // send_ack_time_gap
        0x05, // numDelayedAcks=5 in the low nibble, scale=0 in the high
              // missing 5 addition bytes
    ];
    let result: DecodeResult<AckPayload> = decode(&bytes);
    assert!(result.is_err());
}

// ── AckVectorEntry tests ──

#[test]
fn state_map_entry() {
    let entry = AckVectorEntry::StateMap { bitmap: 0b0110_1010 };
    let byte = entry.to_byte();
    assert_eq!(byte & 0x80, 0, "MSB should be 0 for state-map");
    assert_eq!(byte, 0b0110_1010);
    let decoded = AckVectorEntry::from_byte(byte);
    assert_eq!(decoded, entry);
}

#[test]
fn run_length_received() {
    let entry = AckVectorEntry::RunLength {
        received: true,
        length: 42,
    };
    let byte = entry.to_byte();
    assert_eq!(byte, 0x80 | 0x40 | 42); // MSB=1, state=1, length=42
    let decoded = AckVectorEntry::from_byte(byte);
    assert_eq!(decoded, entry);
}

#[test]
fn run_length_not_received() {
    let entry = AckVectorEntry::RunLength {
        received: false,
        length: 7,
    };
    let byte = entry.to_byte();
    assert_eq!(byte, 0x80 | 7); // MSB=1, state=0, length=7
    let decoded = AckVectorEntry::from_byte(byte);
    assert_eq!(decoded, entry);
}

#[test]
fn run_length_max() {
    let entry = AckVectorEntry::RunLength {
        received: true,
        length: 63, // 6-bit max
    };
    let byte = entry.to_byte();
    assert_eq!(byte, 0xFF); // 0x80 | 0x40 | 0x3F
    let decoded = AckVectorEntry::from_byte(byte);
    assert_eq!(decoded, entry);
}

#[test]
fn entry_coverage() {
    let map = AckVectorEntry::StateMap { bitmap: 0x7F };
    assert_eq!(map.coverage(), 7);

    let run = AckVectorEntry::RunLength {
        received: true,
        length: 30,
    };
    assert_eq!(run.coverage(), 30);
}

// ── AckVectorPayload tests ──

#[test]
fn ack_vector_no_timestamp() {
    let payload = AckVectorPayload {
        base_seq_num: 0x0100,
        timestamp: None,
        send_ack_time_gap_ms: None,
        entries: vec![
            AckVectorEntry::RunLength {
                received: true,
                length: 10,
            },
            AckVectorEntry::StateMap { bitmap: 0b010_1010 },
        ],
    };
    let encoded = encode_vec(&payload).expect("encode");
    assert_eq!(payload.size(), 3 + 2); // 3 fixed + 2 entries
    assert_eq!(encoded.len(), 5);

    let decoded: AckVectorPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded, payload);
}

#[test]
fn ack_vector_with_timestamp() {
    let payload = AckVectorPayload {
        base_seq_num: 0x0042,
        timestamp: Some(0x00AB_CDEF),
        send_ack_time_gap_ms: Some(25),
        entries: vec![AckVectorEntry::RunLength {
            received: true,
            length: 20,
        }],
    };
    let encoded = encode_vec(&payload).expect("encode");
    assert_eq!(payload.size(), 3 + 4 + 1); // 3 fixed + 4 timestamp block + 1 entry
    assert_eq!(encoded.len(), 8);
    // Exact bytes, not just length: a round trip alone would not catch the
    // timestamp and gap fields being swapped symmetrically in both encode
    // and decode. base_seq_num=0x0042, packed=0x81 (1 entry, timestamp
    // present), timestamp 0x00ABCDEF little-endian is EF CD AB, gap=25=0x19,
    // entry RunLength{received:true, length:20} is 0xD4.
    assert_eq!(encoded.as_slice(), &[0x42, 0x00, 0x81, 0xEF, 0xCD, 0xAB, 0x19, 0xD4]);

    let decoded: AckVectorPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded, payload);
}

#[test]
fn ack_vector_empty_entries() {
    let payload = AckVectorPayload {
        base_seq_num: 0,
        timestamp: None,
        send_ack_time_gap_ms: None,
        entries: Vec::new(),
    };
    let encoded = encode_vec(&payload).expect("encode");
    assert_eq!(payload.size(), 3);

    let decoded: AckVectorPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded, payload);
}

#[test]
fn ack_vector_roundtrip_mixed_entries() {
    let payload = AckVectorPayload {
        base_seq_num: 500,
        timestamp: Some(0x00_AABBCC),
        send_ack_time_gap_ms: Some(100),
        entries: vec![
            AckVectorEntry::RunLength {
                received: true,
                length: 63,
            },
            AckVectorEntry::StateMap { bitmap: 0b0111_1111 },
            AckVectorEntry::RunLength {
                received: false,
                length: 1,
            },
            AckVectorEntry::RunLength {
                received: true,
                length: 5,
            },
        ],
    };
    let encoded = encode_vec(&payload).expect("encode");
    let decoded: AckVectorPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded, payload);
}

#[test]
fn ack_vector_rejects_a_state_map_bitmap_above_the_7_bit_maximum() {
    let payload = AckVectorPayload {
        base_seq_num: 0,
        timestamp: None,
        send_ack_time_gap_ms: None,
        entries: vec![AckVectorEntry::StateMap { bitmap: 0x80 }],
    };
    assert!(encode_vec(&payload).is_err());
}

#[test]
fn ack_vector_rejects_a_run_length_above_the_6_bit_maximum() {
    let payload = AckVectorPayload {
        base_seq_num: 0,
        timestamp: None,
        send_ack_time_gap_ms: None,
        entries: vec![AckVectorEntry::RunLength {
            received: true,
            length: 64,
        }],
    };
    assert!(encode_vec(&payload).is_err());
}

#[test]
fn ack_vector_rejects_a_timestamp_without_a_gap() {
    let payload = AckVectorPayload {
        base_seq_num: 0,
        timestamp: Some(0x00_AABBCC),
        send_ack_time_gap_ms: None,
        entries: vec![],
    };
    assert!(encode_vec(&payload).is_err());
}

#[test]
fn ack_vector_rejects_a_gap_without_a_timestamp() {
    let payload = AckVectorPayload {
        base_seq_num: 0,
        timestamp: None,
        send_ack_time_gap_ms: Some(100),
        entries: vec![],
    };
    assert!(encode_vec(&payload).is_err());
}

#[test]
fn ack_vector_insufficient_bytes() {
    let bytes = [0x00, 0x00]; // only 2 bytes, need 3
    let result: DecodeResult<AckVectorPayload> = decode(&bytes);
    assert!(result.is_err());
}

#[test]
fn ack_vector_insufficient_entries() {
    // Claims 10 entries but provides none
    let bytes = [
        0x00, 0x00, // base_seq_num
        10,   // codedAckVecSize=10, TimeStampPresent=0
    ];
    let result: DecodeResult<AckVectorPayload> = decode(&bytes);
    assert!(result.is_err());
}

#[test]
fn ack_vector_insufficient_timestamp_block() {
    // TimeStampPresent=1 but not enough bytes for the timestamp block
    let bytes = [
        0x00, 0x00, // base_seq_num
        0x80, // codedAckVecSize=0, TimeStampPresent=1
        0x05, // only 1 byte of the 4-byte timestamp block
    ];
    let result: DecodeResult<AckVectorPayload> = decode(&bytes);
    assert!(result.is_err());
}

/// The delayed-ACK count is carried in a 4-bit nibble, so a vector that cannot
/// be expressed must be rejected rather than truncated.
///
/// Truncating produced a packet whose header promised delayed ACKs the encoder
/// never wrote, which a peer would then read out of the following payload.
/// Found by the `rdpeudp_pdu_round_trip` fuzz oracle.
#[test]
fn ack_payload_rejects_more_delayed_acks_than_the_nibble_holds() {
    let ack = AckPayload {
        seq_num: 0,
        received_ts: 0,
        send_ack_time_gap: 0,
        delay_ack_time_scale: 0,
        delay_ack_time_additions: vec![0; 16],
    };

    encode_vec(&ack).expect_err("16 delayed ACKs do not fit in a 4-bit count");
}

/// The count on the wire always matches the vector that was encoded.
#[test]
fn ack_payload_count_follows_the_vector() {
    for len in [0usize, 1, 7, 15] {
        let ack = AckPayload {
            seq_num: 1,
            received_ts: 2,
            send_ack_time_gap: 3,
            delay_ack_time_scale: 4,
            delay_ack_time_additions: vec![0xAA; len],
        };

        let encoded = encode_vec(&ack).expect("encode");
        assert_eq!(
            usize::from(encoded[6] & 0x0F),
            len,
            "packed count disagrees with the vector"
        );

        let decoded: AckPayload = decode(&encoded).expect("decode");
        assert_eq!(decoded, ack, "round-trip changed the value");
    }
}
