//! Codec tests for the Network Auto-Detect PDUs ([MS-RDPBCGR] 2.2.14).

use ironrdp_core::{Encode as _, decode, encode_vec};
use ironrdp_pdu::rdp::autodetect::{AutoDetectRequest, BW_STOP_CONNECT_TIME, BW_STOP_LOSSY_UDP, BW_STOP_RELIABLE_UDP};

/// Round-trip a request and hand back what came out plus the bytes it produced.
fn round_trip(request: &AutoDetectRequest) -> (AutoDetectRequest, Vec<u8>) {
    let encoded = encode_vec(request).expect("encode");
    let decoded = decode::<AutoDetectRequest>(&encoded).expect("decode");
    (decoded, encoded)
}

#[test]
fn connect_time_stop_without_a_payload_round_trips() {
    // MS-RDPBCGR 2.2.14.1.4 puts `payloadLength` on the connect-time stop
    // unconditionally, so the encoder must emit it even when no payload was supplied.
    // Omitting it produced bytes the decoder rejected, because the decoder reads that
    // field back for every `BW_STOP_CONNECT_TIME`.
    let request = AutoDetectRequest::BandwidthMeasureStop {
        sequence_number: 7,
        request_type: BW_STOP_CONNECT_TIME,
        payload: None,
    };

    let (decoded, encoded) = round_trip(&request);

    assert_eq!(encoded.len(), request.size(), "size() must agree with encode()");
    match decoded {
        AutoDetectRequest::BandwidthMeasureStop {
            sequence_number,
            request_type,
            payload,
        } => {
            assert_eq!(sequence_number, 7);
            assert_eq!(request_type, BW_STOP_CONNECT_TIME);
            // An absent payload is indistinguishable on the wire from an empty one,
            // so it comes back as `Some(empty)`. The bytes are what has to be stable.
            assert_eq!(payload.as_deref(), Some(&[][..]));
        }
        other => panic!("expected a BandwidthMeasureStop, got {other:?}"),
    }
}

#[test]
fn udp_stop_carrying_a_payload_does_not_emit_it() {
    // The UDP stops have no payload fields at all. Emitting them wrote bytes the
    // decoder never reads back, so the payload was silently dropped on the way through
    // rather than rejected: a round trip that loses data without erroring.
    for request_type in [BW_STOP_RELIABLE_UDP, BW_STOP_LOSSY_UDP] {
        let request = AutoDetectRequest::BandwidthMeasureStop {
            sequence_number: 3,
            request_type,
            payload: Some(vec![0xAA; 16]),
        };

        let (decoded, encoded) = round_trip(&request);

        assert_eq!(
            encoded.len(),
            6,
            "a UDP stop is header-only; a payload must not reach the wire"
        );
        assert_eq!(encoded.len(), request.size(), "size() must agree with encode()");
        assert!(
            matches!(decoded, AutoDetectRequest::BandwidthMeasureStop { payload: None, .. }),
            "the decoder yields no payload for a UDP stop, so encode must not write one"
        );
    }
}

#[test]
fn connect_time_stop_preserves_its_payload() {
    let request = AutoDetectRequest::BandwidthMeasureStop {
        sequence_number: 9,
        request_type: BW_STOP_CONNECT_TIME,
        payload: Some(vec![1, 2, 3, 4]),
    };

    let (decoded, encoded) = round_trip(&request);

    assert_eq!(encoded.len(), request.size());
    assert!(matches!(
        decoded,
        AutoDetectRequest::BandwidthMeasureStop { ref payload, .. } if payload.as_deref() == Some(&[1, 2, 3, 4][..])
    ));
}

#[test]
fn encoding_a_stop_is_stable_across_a_second_round_trip() {
    // What the wire form has to guarantee is byte stability, since the type can express
    // states the protocol cannot (an absent payload on a connect-time stop, a present
    // one on a UDP stop). Decoding then re-encoding must reproduce the same bytes.
    for request in [
        AutoDetectRequest::BandwidthMeasureStop {
            sequence_number: 1,
            request_type: BW_STOP_CONNECT_TIME,
            payload: None,
        },
        AutoDetectRequest::BandwidthMeasureStop {
            sequence_number: 2,
            request_type: BW_STOP_RELIABLE_UDP,
            payload: Some(vec![9; 8]),
        },
    ] {
        let (decoded, first) = round_trip(&request);
        let second = encode_vec(&decoded).expect("re-encode");
        assert_eq!(first, second, "decode then encode must reproduce the same bytes");
    }
}
