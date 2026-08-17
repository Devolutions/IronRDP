use ironrdp_core::{DecodeResult, Encode as _, decode, encode_vec};
use ironrdp_rdpeudp::pdu::*;
// -- SynDataPayload tests --

// Network byte order, per [MS-RDPEUDP] 2.2.
const SYNDATA_BYTES: [u8; 8] = [
    0x12, 0x34, 0x56, 0x78, // snInitialSequenceNumber = 0x12345678
    0x04, 0xD0, // uUpStreamMtu = 1232
    0x04, 0x6C, // uDownStreamMtu = 1132
];

fn syndata() -> SynDataPayload {
    SynDataPayload {
        initial_sequence_number: 0x1234_5678,
        upstream_mtu: 1232,
        downstream_mtu: 1132,
    }
}

#[test]
fn encode_syndata() {
    let encoded = encode_vec(&syndata()).expect("encode");
    assert_eq!(encoded.as_slice(), &SYNDATA_BYTES);
}

#[test]
fn decode_syndata() {
    let decoded: SynDataPayload = decode(&SYNDATA_BYTES).expect("decode");
    assert_eq!(decoded, syndata());
}

#[test]
fn syndata_roundtrip() {
    let original = syndata();
    let encoded = encode_vec(&original).expect("encode");
    let decoded: SynDataPayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn syndata_size() {
    assert_eq!(syndata().size(), 8);
}

#[test]
fn syndata_mtu_below_minimum() {
    let mut bad = SYNDATA_BYTES;
    // Set uUpStreamMtu to 1000 (below 1132)
    bad[4] = 0xE8;
    bad[5] = 0x03;
    let result: DecodeResult<SynDataPayload> = decode(&bad);
    assert!(result.is_err());
}

#[test]
fn syndata_mtu_above_maximum() {
    let mut bad = SYNDATA_BYTES;
    // Set uDownStreamMtu to 2000 (above 1232)
    bad[6] = 0xD0;
    bad[7] = 0x07;
    let result: DecodeResult<SynDataPayload> = decode(&bad);
    assert!(result.is_err());
}

#[test]
fn syndata_encode_rejects_mtu_below_minimum() {
    let bad = SynDataPayload {
        upstream_mtu: MTU_MIN - 1,
        ..syndata()
    };
    let result = encode_vec(&bad);
    assert!(result.is_err());
}

#[test]
fn syndata_encode_rejects_mtu_above_maximum() {
    let bad = SynDataPayload {
        downstream_mtu: MTU_MAX + 1,
        ..syndata()
    };
    let result = encode_vec(&bad);
    assert!(result.is_err());
}

#[test]
fn syndata_mtu_boundary_values() {
    // Both at minimum
    let min_mtu = SynDataPayload {
        initial_sequence_number: 1,
        upstream_mtu: MTU_MIN,
        downstream_mtu: MTU_MIN,
    };
    let encoded = encode_vec(&min_mtu).expect("encode");
    let decoded: SynDataPayload = decode(&encoded).expect("decode");
    assert_eq!(min_mtu, decoded);

    // Both at maximum
    let max_mtu = SynDataPayload {
        initial_sequence_number: 1,
        upstream_mtu: MTU_MAX,
        downstream_mtu: MTU_MAX,
    };
    let encoded = encode_vec(&max_mtu).expect("encode");
    let decoded: SynDataPayload = decode(&encoded).expect("decode");
    assert_eq!(max_mtu, decoded);
}

// -- SynDataExPayload tests --

#[test]
fn encode_syndataex_v2_no_cookie() {
    let payload = SynDataExPayload {
        syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
        udp_ver: UdpVersion::V2,
        cookie_hash: None,
    };
    let encoded = encode_vec(&payload).expect("encode");
    assert_eq!(
        encoded.as_slice(),
        &[
            0x00, 0x01, // uSynExFlags = VERSION_INFO_VALID
            0x00, 0x02, // uUdpVer = V2
        ]
    );
    assert_eq!(payload.size(), 4);
}

#[test]
fn decode_syndataex_v2_no_cookie() {
    let bytes = [0x00, 0x01, 0x00, 0x02];
    let decoded: SynDataExPayload = decode(&bytes).expect("decode");
    assert_eq!(decoded.udp_ver, UdpVersion::V2);
    assert!(decoded.cookie_hash.is_none());
}

#[test]
fn encode_syndataex_v3_with_cookie() {
    let mut cookie = [0u8; 32];
    for (i, byte) in cookie.iter_mut().enumerate() {
        *byte = u8::try_from(i % 256).expect("modulo 256 fits in u8");
    }

    let payload = SynDataExPayload {
        syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
        udp_ver: UdpVersion::V3,
        cookie_hash: Some(cookie),
    };
    let encoded = encode_vec(&payload).expect("encode");
    assert_eq!(payload.size(), 36); // 4 + 32
    assert_eq!(encoded.len(), 36);

    // Verify cookie hash bytes
    assert_eq!(&encoded[4..36], &cookie);
}

#[test]
fn decode_syndataex_v3_with_cookie() {
    let mut bytes = vec![
        0x01, 0x00, // VERSION_INFO_VALID
        0x01, 0x01, // V3 = 0x0101
    ];
    let cookie: Vec<u8> = (0..32).collect();
    bytes.extend_from_slice(&cookie);

    let decoded: SynDataExPayload = decode(&bytes).expect("decode");
    assert_eq!(decoded.udp_ver, UdpVersion::V3);
    let hash = decoded.cookie_hash.expect("cookie hash should be present");
    assert_eq!(hash.as_slice(), cookie.as_slice());
}

#[test]
fn syndataex_roundtrip_v2() {
    let original = SynDataExPayload {
        syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
        udp_ver: UdpVersion::V2,
        cookie_hash: None,
    };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: SynDataExPayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn syndataex_roundtrip_v3_with_cookie() {
    let original = SynDataExPayload {
        syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
        udp_ver: UdpVersion::V3,
        cookie_hash: Some([0xAB; 32]),
    };
    let encoded = encode_vec(&original).expect("encode");
    let decoded: SynDataExPayload = decode(&encoded).expect("decode");
    assert_eq!(original, decoded);
}

/// MS-RDPEUDP 1.7 and 3.1.5.1.3 require a responder to negotiate down to a
/// version it supports when the peer advertises one it does not recognize.
/// Decoding an unrecognized `uUdpVer` must therefore succeed and hand back
/// the raw value, not fail the datagram outright.
#[test]
fn syndataex_unknown_version_decodes_to_the_raw_value() {
    let bytes = [
        0x01, 0x00, // VERSION_INFO_VALID
        0xFF, 0xFF, // unknown version
    ];
    let decoded: SynDataExPayload = decode(&bytes).expect("decode");
    assert_eq!(decoded.udp_ver, UdpVersion(0xFFFF));
    assert!(!decoded.udp_ver.uses_v2_wire_format());
}

#[test]
fn syndataex_insufficient_bytes() {
    let bytes = [0x01, 0x00]; // only 2 bytes, need 4
    let result: DecodeResult<SynDataExPayload> = decode(&bytes);
    assert!(result.is_err());
}

// -- UdpVersion tests --

#[test]
fn version_wire_values() {
    assert_eq!(UdpVersion::V1.0, 0x0001);
    assert_eq!(UdpVersion::V2.0, 0x0002);
    assert_eq!(UdpVersion::V3.0, 0x0101);
}

/// The enum this replaced derived `Hash`; downstream `HashMap`/`HashSet` use
/// keyed on `UdpVersion` must keep working across the representation change.
#[test]
fn version_is_still_usable_as_a_hash_key() {
    let mut supported = std::collections::HashSet::new();
    supported.insert(UdpVersion::V1);
    supported.insert(UdpVersion::V3);

    assert!(supported.contains(&UdpVersion::V1));
    assert!(!supported.contains(&UdpVersion::V2));
}

/// Only version 3 selects the MS-RDPEUDP2 data transfer.
///
/// The name of version 2 suggests otherwise. [MS-RDPEUDP] 1.3.2.2 is explicit:
/// the MS-RDPEUDP data transfer messages "MUST be used only when the version
/// negotiated in the UDP connection initialization phase is version 1 or
/// version 2", and the 2.2.2.9 table mentions [MS-RDPEUDP2] on the 0x0101 row
/// alone.
#[test]
fn only_version_3_selects_the_v2_wire_format() {
    assert!(!UdpVersion::V1.uses_v2_wire_format());
    assert!(!UdpVersion::V2.uses_v2_wire_format());
    assert!(UdpVersion::V3.uses_v2_wire_format());
}

/// The cookie hash rides only on version 3, so any other version carrying one
/// must be rejected rather than encoded and silently dropped on the way back.
///
/// The decoder reads the hash only for version 3; the encoder used to write it
/// for any version, so 32 bytes went out that no peer would read back.
/// Found while fuzzing the round trip during development; the
/// `rdpeudp_pdu_round_trip` oracle that caught it is filed separately.
#[test]
fn syn_data_ex_rejects_a_cookie_hash_without_version_3() {
    let payload = SynDataExPayload {
        syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
        udp_ver: UdpVersion::V2,
        cookie_hash: Some([0u8; 32]),
    };

    encode_vec(&payload).expect_err("a cookie hash cannot ride on version 2");
}

/// Version 3 with a cookie hash round-trips.
#[test]
fn syn_data_ex_round_trips_a_version_3_cookie_hash() {
    let payload = SynDataExPayload {
        syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
        udp_ver: UdpVersion::V3,
        cookie_hash: Some([0xAB; 32]),
    };

    let encoded = encode_vec(&payload).expect("encode");
    let decoded: SynDataExPayload = decode(&encoded).expect("decode");
    assert_eq!(decoded, payload);
}

/// The SYNDATA payload from the [MS-RDPEUDP] 4.1.1 SYN capture.
#[test]
fn decode_the_spec_syndata_capture() {
    // 00 00 00 42 04 D0 04 D0, documented as snInitialSequenceNumber 0x42 and
    // both MTUs 0x04D0 = 1232.
    const CAPTURE: [u8; 8] = [0x00, 0x00, 0x00, 0x42, 0x04, 0xD0, 0x04, 0xD0];

    let payload: SynDataPayload = decode(&CAPTURE).expect("decode");
    assert_eq!(payload.initial_sequence_number, 0x42);
    assert_eq!(payload.upstream_mtu, 1232);
    assert_eq!(payload.downstream_mtu, 1232);

    assert_eq!(encode_vec(&payload).expect("encode"), CAPTURE);
}
