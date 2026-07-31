//! Codec tests for the Network Auto-Detect PDUs ([MS-RDPBCGR] 2.2.14).

use ironrdp_core::{Encode as _, decode, encode_vec};
use ironrdp_pdu::rdp::autodetect::{
    AutoDetectRequest, BW_STOP_CONNECT_TIME, BW_STOP_LOSSY_UDP, BW_STOP_RELIABLE_UDP, NETCHAR_RESULT_ALL,
    NETCHAR_RESULT_BW_RTT, NETCHAR_RESULT_RTT,
};

/// Round-trip a request and hand back what came out plus the bytes it produced.
fn round_trip(request: &AutoDetectRequest) -> (AutoDetectRequest, Vec<u8>) {
    let encoded = encode_vec(request).expect("encode");
    let decoded = decode::<AutoDetectRequest>(&encoded).expect("decode");
    (decoded, encoded)
}

/// MS-RDPBCGR 2.2.14.1.4 does not merely make `payloadLength` present for the
/// connect-time stop, it requires a value "greater than zero". So an absent or
/// empty payload has no conforming encoding and must be refused rather than
/// written as a zero length.
#[test]
fn connect_time_stop_without_a_payload_is_refused() {
    for (label, payload) in [("absent", None), ("empty", Some(Vec::new()))] {
        let request = AutoDetectRequest::BandwidthMeasureStop {
            sequence_number: 7,
            request_type: BW_STOP_CONNECT_TIME,
            payload,
        };

        assert!(
            encode_vec(&request).is_err(),
            "an {label} payload on a connect-time stop must not encode"
        );
    }
}

/// The decoder enforces the same rule, so the two sides agree on what the wire
/// permits. Accepting a zero length here would mean accepting what we refuse to
/// emit.
#[test]
fn connect_time_stop_with_a_zero_payload_length_is_rejected() {
    // A well-formed connect-time stop, then its payloadLength forced to zero.
    let valid = AutoDetectRequest::BandwidthMeasureStop {
        sequence_number: 7,
        request_type: BW_STOP_CONNECT_TIME,
        payload: Some(vec![0xAA; 4]),
    };
    let mut wire = encode_vec(&valid).expect("encode");

    // headerLength(1) + headerTypeId(1) + sequenceNumber(2) + requestType(2) = 6,
    // so payloadLength is the u16 at offset 6.
    wire[6] = 0;
    wire[7] = 0;
    wire.truncate(8);

    assert!(decode::<AutoDetectRequest>(&wire).is_err());
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
    // What the wire form has to guarantee is byte stability, since the type can still
    // express one state the protocol cannot: a payload on a UDP stop, which is dropped
    // rather than emitted. (The other such state, an absent payload on a connect-time
    // stop, is now refused outright rather than normalised, so it has no bytes to be
    // stable about.) Decoding then re-encoding must reproduce the same bytes.
    for request in [
        AutoDetectRequest::BandwidthMeasureStop {
            sequence_number: 1,
            request_type: BW_STOP_CONNECT_TIME,
            payload: Some(vec![7; 3]),
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

/// MS-RDPBCGR 2.2.14.1.5 assigns the optional fields by `requestType`: 0x0840
/// carries baseRTT, 0x0880 carries bandwidth, 0x08C0 carries both. The decoder
/// already read them back on that basis, so the encoder must write them on the
/// same basis or the two disagree about the wire.
#[test]
fn netchar_result_fields_follow_the_request_type() {
    for (label, request_type, base_rtt, bandwidth) in [
        ("RTT", NETCHAR_RESULT_RTT, Some(11), None),
        ("BW_RTT", NETCHAR_RESULT_BW_RTT, None, Some(22)),
        ("ALL", NETCHAR_RESULT_ALL, Some(33), Some(44)),
    ] {
        let request = AutoDetectRequest::NetworkCharacteristicsResult {
            sequence_number: 5,
            request_type,
            base_rtt_ms: base_rtt,
            bandwidth_kbps: bandwidth,
            average_rtt_ms: 99,
        };

        let (decoded, encoded) = round_trip(&request);
        assert_eq!(
            encoded.len(),
            request.size(),
            "{label}: size() must agree with encode()"
        );
        assert_eq!(decoded, request, "{label}: must survive a round trip");
    }
}

/// A value the `requestType` does not call for is dropped rather than written.
///
/// This is the case that distinguishes the two rules: with the fields keyed off
/// the `Option`s, a `NETCHAR_RESULT_RTT` carrying a bandwidth wrote twelve bytes
/// of body that the decoder reads as eight, so the extra value both corrupted
/// the frame and disagreed with `headerLength`, which was always derived from
/// `requestType`.
#[test]
fn netchar_result_drops_a_value_its_request_type_does_not_carry() {
    let extra = AutoDetectRequest::NetworkCharacteristicsResult {
        sequence_number: 5,
        request_type: NETCHAR_RESULT_RTT,
        base_rtt_ms: Some(11),
        bandwidth_kbps: Some(22), // not carried by 0x0840
        average_rtt_ms: 99,
    };

    let (decoded, encoded) = round_trip(&extra);

    assert_eq!(encoded.len(), extra.size(), "size() must agree with encode()");
    match decoded {
        AutoDetectRequest::NetworkCharacteristicsResult {
            base_rtt_ms,
            bandwidth_kbps,
            average_rtt_ms,
            ..
        } => {
            assert_eq!(base_rtt_ms, Some(11));
            assert_eq!(bandwidth_kbps, None, "the uncarried bandwidth must not reach the wire");
            assert_eq!(
                average_rtt_ms, 99,
                "averageRTT must not be displaced by the extra value"
            );
        }
        other => panic!("expected a NetworkCharacteristicsResult, got {other:?}"),
    }
}

/// A value the `requestType` does not call for is not silently written into the
/// slot of one it does. Before this, a `NETCHAR_RESULT_RTT` carrying only a
/// bandwidth wrote that bandwidth where the decoder expects baseRTT, so the
/// value came back corrupted rather than rejected.
#[test]
fn netchar_result_rejects_a_payload_that_contradicts_its_request_type() {
    let mismatched = AutoDetectRequest::NetworkCharacteristicsResult {
        sequence_number: 5,
        request_type: NETCHAR_RESULT_RTT,
        base_rtt_ms: None,
        bandwidth_kbps: Some(22),
        average_rtt_ms: 99,
    };

    assert!(
        encode_vec(&mismatched).is_err(),
        "a baseRTT-carrying request type with no baseRTT must not encode"
    );
}
