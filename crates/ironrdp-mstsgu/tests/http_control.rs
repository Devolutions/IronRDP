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

use ironrdp_core::{Decode, Encode, ReadCursor, WriteCursor};
use ironrdp_mstsgu::{ChannelClosePkt, ReauthMessagePkt, ServiceMessagePkt, gateway_code_label};
use proto::TunnelReqPkt;

fn encode_to_vec(payload: &impl Encode) -> Vec<u8> {
    let mut buf = vec![0u8; payload.size()];
    let mut cur = WriteCursor::new(&mut buf);
    payload.encode(&mut cur).expect("encode");
    assert_eq!(cur.pos(), payload.size());
    buf
}

fn decode_body<'a, T: Decode<'a>>(bytes: &'a [u8]) -> T {
    // HTTP_PACKET_HEADER is 8 bytes and is stripped before body decode.
    let mut cur = ReadCursor::new(&bytes[8..]);
    let decoded = T::decode(&mut cur).expect("decode");
    assert!(cur.eof());
    decoded
}

#[test]
fn tunnel_request_without_reauth_context_preserves_existing_wire_format() {
    let bytes = encode_to_vec(&TunnelReqPkt {
        caps: 0x04,
        fields_present: 0,
        ..TunnelReqPkt::default()
    });

    assert_eq!(
        bytes,
        [
            0x04, 0x00, 0x00, 0x00, // packetType, reserved
            0x10, 0x00, 0x00, 0x00, // packet length
            0x04, 0x00, 0x00, 0x00, // capsFlags
            0x00, 0x00, // fieldsPresent
            0x00, 0x00, // reserved
        ]
    );
}

#[test]
fn tunnel_request_encodes_reauth_context() {
    let reauth_tunnel_context = 0x1122_3344_5566_7788;
    let bytes = encode_to_vec(&TunnelReqPkt {
        caps: 0x04,
        reauth_tunnel_context: Some(reauth_tunnel_context),
        ..TunnelReqPkt::default()
    });

    assert_eq!(
        bytes.len(),
        8 /* HTTP_PACKET_HEADER */
            + 4 /* capsFlags */
            + 2 /* fieldsPresent */
            + 2 /* reserved */
            + 8 /* reauthTunnelContext */
    );
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 24);
    assert_eq!(&bytes[12..14], &0x2u16.to_le_bytes());
    assert_eq!(&bytes[16..24], &reauth_tunnel_context.to_le_bytes());
}

#[test]
fn service_message_roundtrip() {
    let pkt = ServiceMessagePkt {
        message: "OK".to_owned(),
    };
    let bytes = encode_to_vec(&pkt);
    assert_eq!(&bytes[..2], &0x0Bu16.to_le_bytes());
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        u32::try_from(bytes.len()).unwrap()
    );
    let decoded = decode_body::<ServiceMessagePkt>(&bytes);
    assert_eq!(decoded, pkt);
}

#[test]
fn reauth_message_roundtrip() {
    let pkt = ReauthMessagePkt {
        reauth_tunnel_context: 0x1122_3344_5566_7788,
    };
    let bytes = encode_to_vec(&pkt);
    assert_eq!(&bytes[..2], &0x0Cu16.to_le_bytes());
    assert_eq!(
        bytes.len(),
        8 /* HTTP_PACKET_HEADER */ + 8 /* reauthTunnelContext */
    );
    let decoded = decode_body::<ReauthMessagePkt>(&bytes);
    assert_eq!(decoded, pkt);
}

#[test]
fn channel_close_roundtrip() {
    let pkt = ChannelClosePkt {
        status_code: 0x8007_59DA,
    };
    let bytes = encode_to_vec(&pkt);
    assert_eq!(&bytes[..2], &0x10u16.to_le_bytes());
    assert_eq!(bytes.len(), 8 /* HTTP_PACKET_HEADER */ + 4 /* statusCode */);
    let decoded = decode_body::<ChannelClosePkt>(&bytes);
    assert_eq!(decoded, pkt);
}

#[test]
fn gateway_code_label_known_and_unknown() {
    assert_eq!(gateway_code_label(0), Some("ERROR_SUCCESS"));
    assert_eq!(gateway_code_label(0x8007_59DA), Some("E_PROXY_RAP_ACCESSDENIED"));
    assert_eq!(gateway_code_label(0x0000_59DA), Some("E_PROXY_RAP_ACCESSDENIED"));
    assert_eq!(gateway_code_label(0x0000_59DD), Some("E_PROXY_TS_CONNECTFAILED"));
    assert_eq!(gateway_code_label(0x8007_59DD), Some("E_PROXY_TS_CONNECTFAILED"));
    assert_eq!(gateway_code_label(0xDEAD_BEEF), None);
}
