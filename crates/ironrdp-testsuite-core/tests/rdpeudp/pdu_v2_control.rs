use ironrdp_core::{DecodeResult, Encode as _, decode, encode_vec};
use ironrdp_rdpeudp::pdu::*;
// ── OverheadSize tests ──

#[test]
fn overhead_size_roundtrip() {
    let original = OverheadSizePayload { overhead_size: 42 };
    let encoded = encode_vec(&original).expect("encode");
    assert_eq!(encoded.as_slice(), &[42]);
    assert_eq!(original.size(), 1);

    let decoded: OverheadSizePayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn overhead_size_zero() {
    let original = OverheadSizePayload { overhead_size: 0 };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: OverheadSizePayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn overhead_size_max() {
    let original = OverheadSizePayload { overhead_size: 0xFF };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: OverheadSizePayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn overhead_size_insufficient_bytes() {
    let bytes: [u8; 0] = [];
    let result: DecodeResult<OverheadSizePayload> = decode(&bytes);
    assert!(result.is_err());
}

// ── DelayAckInfo tests ──

#[test]
fn delay_ack_info_encode() {
    let payload = DelayAckInfoPayload {
        max_delayed_acks: 8,
        delayed_ack_timeout_ms: 150,
    };
    let encoded = encode_vec(&payload).expect("encode");
    assert_eq!(
        encoded.as_slice(),
        &[
            0x08, // MaxDelayedAcks is a whole byte per 2.2.1.2.3
            0x96, 0x00, // DelayedAckTimeoutInMs = 150, little-endian
        ]
    );
    assert_eq!(payload.size(), 3);
}

#[test]
fn delay_ack_info_decode() {
    let bytes = [0x08, 0x96, 0x00];
    let decoded: DelayAckInfoPayload = decode(&bytes).expect("decode");
    assert_eq!(decoded.max_delayed_acks, 8);
    assert_eq!(decoded.delayed_ack_timeout_ms, 150);
}

#[test]
fn delay_ack_info_roundtrip() {
    let original = DelayAckInfoPayload {
        max_delayed_acks: 15,
        delayed_ack_timeout_ms: 1000,
    };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: DelayAckInfoPayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn delay_ack_info_default_values() {
    let payload = DelayAckInfoPayload {
        max_delayed_acks: DelayAckInfoPayload::DEFAULT_MAX_DELAYED_ACKS,
        delayed_ack_timeout_ms: 50,
    };
    assert_eq!(payload.max_delayed_acks, 8);
}

#[test]
fn delay_ack_info_insufficient_bytes() {
    let bytes = [0x80, 0x96]; // only 2 bytes, need 3
    let result: DecodeResult<DelayAckInfoPayload> = decode(&bytes);
    assert!(result.is_err());
}

// ── AckOfAcks tests ──

#[test]
fn ack_of_acks_roundtrip() {
    let original = AckOfAcksPayload {
        ack_of_acks_seq_num: 0x1234,
    };
    let encoded = encode_vec(&original).expect("encode");
    assert_eq!(encoded.as_slice(), &[0x34, 0x12]); // little-endian
    assert_eq!(original.size(), 2);

    let decoded: AckOfAcksPayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn ack_of_acks_zero() {
    let original = AckOfAcksPayload { ack_of_acks_seq_num: 0 };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: AckOfAcksPayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn ack_of_acks_max() {
    let original = AckOfAcksPayload {
        ack_of_acks_seq_num: 0xFFFF,
    };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: AckOfAcksPayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn ack_of_acks_insufficient_bytes() {
    let bytes = [0x34]; // only 1 byte, need 2
    let result: DecodeResult<AckOfAcksPayload> = decode(&bytes);
    assert!(result.is_err());
}

/// MaxDelayedAcks is a full byte, so values above 15 survive the round trip.
///
/// It was previously packed into a nibble, which silently truncated anything
/// larger and made the sender's advertised limit unrepresentable.
#[test]
fn delay_ack_info_carries_a_full_byte_of_max_delayed_acks() {
    for max_delayed_acks in [0u8, 15, 16, 200, 255] {
        let payload = DelayAckInfoPayload {
            max_delayed_acks,
            delayed_ack_timeout_ms: 42,
        };

        let encoded = encode_vec(&payload).expect("encode");
        assert_eq!(encoded[0], max_delayed_acks);
        assert_eq!(decode::<DelayAckInfoPayload>(&encoded).expect("decode"), payload);
    }
}
