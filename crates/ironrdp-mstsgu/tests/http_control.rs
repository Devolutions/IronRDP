#![allow(unused_crate_dependencies)]

use ironrdp_core::{Decode, Encode, ReadCursor, WriteCursor};
use ironrdp_mstsgu::{ChannelClosePkt, ReauthMessagePkt, ServiceMessagePkt, gateway_code_label};

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
