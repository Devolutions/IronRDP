//! Tests for resilient capability-set decoding, per the
//! "Enumeration-like types should allow resilient parsing" section of
//! `crates/ironrdp-pdu/README.md`.
//!
//! Two properties matter:
//!
//! - an unrecognized `CapabilityVersion` decodes into a
//!   `RawCapabilitySet` whose `.parsed()` returns `None`, instead of
//!   failing the whole PDU;
//! - the original wire bytes (including the version value) are preserved so
//!   that `encode(decode(m)) == m`.

use ironrdp_core::{decode, encode_vec};
use ironrdp_egfx::pdu::{CapabilitiesAdvertisePdu, CapabilityVersion};
use proptest::{prelude::*, sample::select};

/// Build a raw `RDPGFX_CAPS_ADVERTISE_PDU` carrying a single capset:
/// `capsSetCount=1` then `version`, `dataLength`, `data`.
fn raw_advertise_one(version: u32, data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(2 + 8 + data.len());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&version.to_le_bytes());
    let len = u32::try_from(data.len()).expect("test data length fits u32");
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(data);
    buf
}

/// Choose a random version with a bias towards well-known ones.
fn version() -> impl Strategy<Value = u32> {
    prop_oneof![
        select::<u32>(&[
            CapabilityVersion::V8.0,
            CapabilityVersion::V8_1.0,
            CapabilityVersion::V10.0,
            CapabilityVersion::V10_1.0,
            CapabilityVersion::V10_2.0,
            CapabilityVersion::V10_3.0,
            CapabilityVersion::V10_4.0,
            CapabilityVersion::V10_5.0,
            CapabilityVersion::V10_6.0,
            CapabilityVersion::V10_6_ERR.0,
            CapabilityVersion::V10_7.0,
        ]),
        any::<u32>(),
    ]
}

/// `encode(decode(wire)) == wire` for any version and any payload.
#[test]
fn capability_set_roundtrips() {
    proptest!(|(
        version in version(),
        data in proptest::collection::vec(any::<u8>(), 0..32usize),
    )| {
        let wire = raw_advertise_one(version, &data);
        let pdu: CapabilitiesAdvertisePdu = decode(&wire).expect("decode must tolerate unknown version");

        let cap = &pdu.0[0];
        if cap.version.is_known() {
            // `parsed()` may fail because of invalid data format (length, etc) for known versions.
            if let Ok(parsed) = cap.parsed() {
                prop_assert!(parsed.is_some());
            }
        } else {
            // `parsed()` never fails for unknown versions.
            prop_assert!(cap.parsed().expect("parsed never errors for unknown versions").is_none());
        }

        let re_encoded = encode_vec(&pdu).expect("encode must succeed");
        prop_assert_eq!(re_encoded, wire);
    });
}
