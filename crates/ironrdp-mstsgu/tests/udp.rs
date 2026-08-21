use ironrdp_core::{Decode as _, Encode, ReadCursor, WriteCursor};
use ironrdp_mstsgu::{
    AaSynData, AaSynDataResp, ConnectPkt, ConnectPktResp, DataPkt, DiscPkt, GwUdpOffer, MAX_CONNECT_REQ_FRAGMENT_SIZE,
    UdpCorrelationInfo, UdpPacketHeader, UdpPktType, encode_connect_request, fragment_connect_pkt,
};

fn encode_to_vec(payload: &impl Encode) -> Vec<u8> {
    let mut buf = vec![0u8; payload.size()];
    let mut cur = WriteCursor::new(&mut buf);
    payload.encode(&mut cur).expect("encode");
    assert_eq!(cur.pos(), payload.size());
    buf
}

#[test]
fn connect_pkt_roundtrip() {
    let offer = GwUdpOffer {
        port: 3391,
        authn_cookie: vec![1, 2, 3, 4, 5],
    };
    let syn = AaSynData {
        up_stream_mtu: 1234,
        down_stream_mtu: 1400,
        lossy: 1,
        send_isn: 42,
    };
    let pkt = ConnectPkt::from_offer(&offer, 3389, syn);
    let bytes = encode_to_vec(&pkt);
    assert_eq!(&bytes[..2], &UdpPktType::ConnectReq.as_u16().to_le_bytes());
    let mut cur = ReadCursor::new(&bytes);
    let decoded = ConnectPkt::decode(&mut cur).expect("decode");
    assert_eq!(decoded.target_port, 3389);
    assert_eq!(decoded.syn_data, syn);
    assert_eq!(decoded.authn_cookie, offer.authn_cookie);
    assert!(cur.eof());
}

#[test]
fn connect_pkt_resp_roundtrip() {
    let pkt = ConnectPktResp {
        syn_response: AaSynDataResp {
            up_stream_mtu: 1200,
            down_stream_mtu: 1300,
            recv_isn: 7,
        },
        result: 0,
    };
    let bytes = encode_to_vec(&pkt);
    assert_eq!(&bytes[..2], &UdpPktType::ConnectResp.as_u16().to_le_bytes());
    let mut cur = ReadCursor::new(&bytes);
    let decoded = ConnectPktResp::decode(&mut cur).expect("decode");
    assert_eq!(decoded, pkt);
    assert!(cur.eof());
}

#[test]
fn data_pkt_roundtrip() {
    let pkt = DataPkt {
        data: b"rdp-udp-payload".to_vec(),
    };
    let bytes = encode_to_vec(&pkt);
    assert_eq!(&bytes[..2], &UdpPktType::Payload.as_u16().to_le_bytes());
    let mut cur = ReadCursor::new(&bytes);
    let decoded = DataPkt::decode(&mut cur).expect("decode");
    assert_eq!(decoded.data, pkt.data);
    assert!(cur.eof());
}

#[test]
fn disc_pkt_roundtrip() {
    let pkt = DiscPkt { reason: 0x0000_04CA };
    let bytes = encode_to_vec(&pkt);
    assert_eq!(&bytes[..2], &UdpPktType::Disconnect.as_u16().to_le_bytes());
    assert_eq!(bytes.len(), UdpPacketHeader::FIXED_PART_SIZE + 8);
    let mut cur = ReadCursor::new(&bytes);
    let decoded = DiscPkt::decode(&mut cur).expect("decode");
    assert_eq!(decoded.reason, pkt.reason);
    assert!(cur.eof());
}

#[test]
fn correlation_info_roundtrip() {
    let id = [9u8; 16];
    let info = UdpCorrelationInfo::new(id);
    let bytes = encode_to_vec(&info);
    assert_eq!(bytes.len(), UdpCorrelationInfo::SIZE);
    let mut cur = ReadCursor::new(&bytes);
    let decoded = UdpCorrelationInfo::decode(&mut cur).expect("decode");
    assert_eq!(decoded.correlation_id, id);
    assert!(cur.eof());
}

#[test]
fn fragment_connect_pkt_covers_full_request() {
    let offer = GwUdpOffer {
        port: 3391,
        authn_cookie: vec![7; MAX_CONNECT_REQ_FRAGMENT_SIZE + 40],
    };
    let pkt = ConnectPkt::from_offer(&offer, 2179, AaSynData::default());
    let frags = fragment_connect_pkt(&pkt).expect("fragment");
    assert!(frags.len() > 1);
    let mut reassembled = Vec::new();
    for (idx, frag) in frags.iter().enumerate() {
        let mut cur = ReadCursor::new(frag);
        let hdr = UdpPacketHeader::decode(&mut cur).unwrap();
        assert_eq!(hdr.pkt_id, UdpPktType::ConnectReqFragment.as_u16());
        let id = cur.read_u16();
        assert_eq!(usize::from(id), idx);
        let total = cur.read_u16();
        assert_eq!(usize::from(total), frags.len());
        let len = usize::from(cur.read_u16());
        if idx + 1 < frags.len() {
            assert_eq!(len, MAX_CONNECT_REQ_FRAGMENT_SIZE);
        }
        reassembled.extend_from_slice(cur.read_slice(len));
    }
    let full = encode_to_vec(&pkt);
    assert_eq!(reassembled, full);
}

#[test]
fn encode_connect_request_matches_struct() {
    let offer = GwUdpOffer {
        port: 1,
        authn_cookie: b"cookie".to_vec(),
    };
    let bytes = encode_connect_request(&offer, 3389, AaSynData::default()).expect("encode");
    let mut cur = ReadCursor::new(&bytes);
    let decoded = ConnectPkt::decode(&mut cur).expect("decode");
    assert_eq!(decoded.authn_cookie, b"cookie");
}

#[test]
fn payload_and_disconnect_reject_wrong_pkt_id() {
    let connect = encode_to_vec(&ConnectPkt {
        target_port: 3389,
        syn_data: AaSynData::default(),
        authn_cookie: vec![1],
    });
    let mut cur = ReadCursor::new(&connect);
    assert!(DataPkt::decode(&mut cur).is_err());
    let mut cur = ReadCursor::new(&connect);
    assert!(DiscPkt::decode(&mut cur).is_err());
}

fn patch_pkt_len(bytes: &mut [u8], pkt_len: u16) {
    bytes[2..4].copy_from_slice(&pkt_len.to_le_bytes());
}

#[test]
fn connect_pkt_rejects_mismatched_pkt_len() {
    let mut bytes = encode_to_vec(&ConnectPkt {
        target_port: 3389,
        syn_data: AaSynData::default(),
        authn_cookie: vec![1, 2, 3],
    });
    patch_pkt_len(&mut bytes, 4);
    let mut cur = ReadCursor::new(&bytes);
    assert!(ConnectPkt::decode(&mut cur).is_err());
}

#[test]
fn connect_pkt_resp_rejects_mismatched_pkt_len() {
    let mut bytes = encode_to_vec(&ConnectPktResp::default());
    patch_pkt_len(&mut bytes, 0);
    let mut cur = ReadCursor::new(&bytes);
    assert!(ConnectPktResp::decode(&mut cur).is_err());
}

#[test]
fn disc_pkt_rejects_mismatched_pkt_len() {
    let mut bytes = encode_to_vec(&DiscPkt { reason: 1 });
    patch_pkt_len(&mut bytes, 0);
    let mut cur = ReadCursor::new(&bytes);
    assert!(DiscPkt::decode(&mut cur).is_err());
}
