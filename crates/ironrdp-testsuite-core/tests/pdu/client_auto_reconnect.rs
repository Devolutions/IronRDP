use ironrdp_core::{DecodeErrorKind, decode, encode_vec};
use ironrdp_pdu::rdp::client_info::ClientAutoReconnect;
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;

fn server_cookie(logon_id: u32, random_bits: [u8; 16]) -> ServerAutoReconnect {
    ServerAutoReconnect { logon_id, random_bits }
}

/// [MS-RDPBCGR] 5.5: `SecurityVerifier = HMAC(AutoReconnectRandom, ClientRandom)`,
/// HMAC-MD5 keyed by the server's random bits. Enhanced RDP Security generates no
/// client random, so the spec substitutes 32 zero bytes.
///
/// The expected value is an independent HMAC-MD5 of 32 zero bytes under the key
/// `00..0f`, so this pins the derivation itself rather than restating the code.
#[test]
fn security_verifier_is_hmac_md5_of_the_zero_client_random() {
    let mut random_bits = [0u8; 16];
    for (i, byte) in random_bits.iter_mut().enumerate() {
        *byte = u8::try_from(i).unwrap();
    }

    let arc_cs = ClientAutoReconnect::from_server_cookie(&server_cookie(7, random_bits));

    assert_eq!(
        arc_cs.security_verifier,
        [
            0xb6, 0x39, 0xc8, 0x73, 0x16, 0x38, 0x61, 0x8b, 0x70, 0x79, 0x72, 0xaa, 0x6e, 0x96, 0xcf, 0x90,
        ]
    );
}

/// A second independent vector, contributed by @clintcan on #1509 alongside a
/// live mstsc validation of this construction: HMAC-MD5 of 32 zero bytes under
/// the key `01..10`. Two vectors under different keys pin the keying, not just
/// the message.
#[test]
fn security_verifier_matches_a_second_reference_vector() {
    let mut random_bits = [0u8; 16];
    for (i, byte) in random_bits.iter_mut().enumerate() {
        *byte = u8::try_from(i + 1).unwrap();
    }

    let arc_cs = ClientAutoReconnect::from_server_cookie(&server_cookie(1, random_bits));

    assert_eq!(
        arc_cs.security_verifier,
        [
            0x89, 0x40, 0x25, 0xa9, 0x9d, 0x64, 0xab, 0x96, 0x64, 0x19, 0xec, 0x1e, 0xf1, 0x3c, 0x26, 0x1c,
        ]
    );
}

/// The server accepts a packet it could have produced itself, and nothing else.
#[test]
fn verify_accepts_the_derived_answer() {
    let cookie = server_cookie(0x1234_5678, [0x3C; 16]);

    assert!(ClientAutoReconnect::from_server_cookie(&cookie).verify(&cookie));
}

/// A single flipped byte in the verifier must fail. This is the check that makes
/// the cookie an authentication factor rather than a hint.
#[test]
fn verify_rejects_a_tampered_verifier() {
    let cookie = server_cookie(0x1234_5678, [0x3C; 16]);
    let mut arc_cs = ClientAutoReconnect::from_server_cookie(&cookie);
    arc_cs.security_verifier[0] ^= 0xFF;

    assert!(!arc_cs.verify(&cookie));
}

/// Both halves have to match. A correct verifier presented against a different
/// session identifier is a different session, so it must not resume this one.
#[test]
fn verify_requires_both_logon_id_and_verifier() {
    let cookie = server_cookie(0x1234_5678, [0x3C; 16]);
    let mut arc_cs = ClientAutoReconnect::from_server_cookie(&cookie);
    arc_cs.logon_id = 1;

    assert!(!arc_cs.verify(&cookie));
}

/// A verifier derived from a different random must not resume the session: this
/// is the case that distinguishes verification from merely parsing the packet.
#[test]
fn verify_rejects_an_answer_to_a_different_cookie() {
    let issued = server_cookie(0x1234_5678, [0x3C; 16]);
    let other = server_cookie(0x1234_5678, [0xC3; 16]);

    assert!(!ClientAutoReconnect::from_server_cookie(&other).verify(&issued));
}

/// The session identifier is echoed from the server's cookie: it tells the server
/// which session is being resumed.
#[test]
fn logon_id_is_carried_over_from_the_server_cookie() {
    let arc_cs = ClientAutoReconnect::from_server_cookie(&server_cookie(0xDEAD_BEEF, [0xAB; 16]));

    assert_eq!(arc_cs.logon_id, 0xDEAD_BEEF);
}

/// [MS-RDPBCGR] 2.2.4.3: cbLen (4) + Version (4) + LogonId (4) + SecurityVerifier
/// (16), with cbLen fixed at 0x1C. Unlike the server packet there is no enclosing
/// logon-info field header, so the encoding is exactly 28 bytes.
#[test]
fn encodes_the_arc_cs_layout() {
    let arc_cs = ClientAutoReconnect {
        logon_id: 0x0000_002A,
        security_verifier: [0x11; 16],
    };

    let encoded = encode_vec(&arc_cs).unwrap();

    assert_eq!(encoded.len(), 28, "cbLen is fixed at 0x0000001C");
    assert_eq!(&encoded[0..4], &0x0000_001Cu32.to_le_bytes(), "cbLen");
    assert_eq!(
        &encoded[4..8],
        &0x0000_0001u32.to_le_bytes(),
        "AUTO_RECONNECT_VERSION_1"
    );
    assert_eq!(&encoded[8..12], &0x0000_002Au32.to_le_bytes(), "LogonId");
    assert_eq!(&encoded[12..28], &[0x11; 16], "SecurityVerifier");

    assert_eq!(
        arc_cs.to_bytes().as_slice(),
        encoded.as_slice(),
        "to_bytes must agree with the Encode impl"
    );
}

#[test]
fn round_trips() {
    let arc_cs = ClientAutoReconnect::from_server_cookie(&server_cookie(9, [0x5A; 16]));

    let encoded = encode_vec(&arc_cs).unwrap();
    let decoded = decode::<ClientAutoReconnect>(&encoded).unwrap();

    assert_eq!(decoded, arc_cs);
}

/// A packet claiming any length other than 0x1C is malformed: the structure is
/// fixed-size, so a differing cbLen means the peer and we disagree about the
/// layout rather than about an optional tail.
///
/// The reported offset must point at cbLen's own position (0), not past it: the
/// check reads cbLen before rejecting it, so a naive `in: src` after the read
/// would report offset 4 instead.
#[test]
fn rejects_a_wrong_packet_length() {
    let mut bytes = ClientAutoReconnect {
        logon_id: 1,
        security_verifier: [0; 16],
    }
    .to_bytes();
    bytes[0] = 0x1D;

    let error = decode::<ClientAutoReconnect>(&bytes).unwrap_err();
    let DecodeErrorKind::InvalidField { offset, .. } = error.kind() else {
        panic!("expected InvalidField, got {:?}", error.kind());
    };
    assert_eq!(*offset, Some(0), "offset must point at cbLen, not past it");
}

/// Only version 1 is defined; anything else would carry a layout we have not been
/// told about.
///
/// The reported offset must point at Version's own position (4), not past it.
#[test]
fn rejects_an_unknown_version() {
    let mut bytes = ClientAutoReconnect {
        logon_id: 1,
        security_verifier: [0; 16],
    }
    .to_bytes();
    bytes[4] = 0x02;

    let error = decode::<ClientAutoReconnect>(&bytes).unwrap_err();
    let DecodeErrorKind::InvalidField { offset, .. } = error.kind() else {
        panic!("expected InvalidField, got {:?}", error.kind());
    };
    assert_eq!(*offset, Some(4), "offset must point at Version, not past it");
}
