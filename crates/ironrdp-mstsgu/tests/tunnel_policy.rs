#![expect(
    dead_code,
    unreachable_pub,
    unused_crate_dependencies,
    reason = "tests import private protocol structures"
)]

#[path = "../src/proto.rs"]
#[expect(
    clippy::allow_attributes,
    reason = "the imported protocol source contains an intentionally unfulfilled expectation"
)]
#[allow(unfulfilled_lint_expectations)]
mod proto;

use ironrdp_core::{Decode as _, ReadCursor};
use ironrdp_mstsgu::GwTunnelPolicy;
use proto::TunnelAuthRespPkt;

#[test]
fn tunnel_auth_response_decodes_optional_policy_fields() {
    let bytes = [
        0x00, 0x00, 0x00, 0x00, // errorCode
        0x07, 0x00, // fieldsPresent
        0x00, 0x00, // reserved
        0x08, 0x00, 0x00, 0x40, // redirFlags
        0x0f, 0x00, 0x00, 0x00, // idleTimeout
        0x03, 0x00, // SoHResponse cbLen
        0x01, 0x02, 0x03, // SoHResponse
    ];

    let mut cursor = ReadCursor::new(&bytes);
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
