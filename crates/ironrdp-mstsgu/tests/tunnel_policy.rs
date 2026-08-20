#![allow(unused_crate_dependencies)]

#[path = "../src/proto.rs"]
#[allow(dead_code, unreachable_pub, unfulfilled_lint_expectations)]
mod proto;

use ironrdp_core::{Decode, Encode, ReadCursor, WriteCursor};
use ironrdp_mstsgu::GwTunnelPolicy;
use proto::TunnelAuthRespPkt;

fn encode_to_vec(payload: &impl Encode) -> Vec<u8> {
    let mut buf = vec![0; payload.size()];
    let mut cursor = WriteCursor::new(&mut buf);
    payload.encode(&mut cursor).expect("encode");
    assert_eq!(cursor.pos(), payload.size());
    buf
}

#[test]
fn tunnel_auth_response_roundtrip_preserves_optional_policy_fields() {
    let response = TunnelAuthRespPkt {
        redirection_flags: Some(0x4000_0008),
        idle_timeout_minutes: Some(15),
        soh_response: Some(vec![0x01, 0x02, 0x03]),
        ..TunnelAuthRespPkt::default()
    };

    let bytes = encode_to_vec(&response);
    assert_eq!(&bytes[..2], &0x07u16.to_le_bytes());
    assert_eq!(&bytes[12..14], &0x07u16.to_le_bytes());
    assert_eq!(
        bytes.len(),
        8 /* HTTP_PACKET_HEADER */
            + 4 /* errorCode */
            + 2 /* fieldsPresent */
            + 2 /* reserved */
            + 4 /* redirFlags */
            + 4 /* idleTimeout */
            + 2 /* cbLen */
            + 3 /* SoHResponse */
    );

    let mut cursor = ReadCursor::new(&bytes[8..]);
    let decoded = TunnelAuthRespPkt::decode(&mut cursor).expect("decode");
    assert!(cursor.eof());
    assert_eq!(decoded.redirection_flags, Some(0x4000_0008));
    assert_eq!(decoded.idle_timeout_minutes, Some(15));
    assert_eq!(decoded.soh_response.as_deref(), Some(&[0x01, 0x02, 0x03][..]));
}

#[test]
fn tunnel_policy_defaults_to_no_gateway_values() {
    assert_eq!(
        GwTunnelPolicy::default(),
        GwTunnelPolicy {
            redirection_flags: None,
            idle_timeout_minutes: None,
            soh_response: None,
        }
    );
}
