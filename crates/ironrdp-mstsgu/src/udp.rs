//! RDG-UDP packet layouts ([MS-TSGU] 2.2.5.4 / 2.2.11).
//!
//! Opening a live side channel still requires DTLS + MS-RDPEUDP; this module encodes and
//! decodes the MS-TSGU UDP framing used after the main HTTP channel supplies a cookie.

use ironrdp_core::{
    Decode, Encode, ReadCursor, WriteCursor, cast_int, ensure_fixed_part_size, ensure_size, other_err,
    unsupported_value_err,
};

/// Parameters from `HTTP_CHANNEL_RESPONSE` that enable a future RDG-UDP side channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GwUdpOffer {
    /// UDP port on the gateway that accepts the side channel.
    pub port: u16,
    /// Opaque authentication cookie for [`CONNECT_PKT`](ConnectPkt).
    pub authn_cookie: Vec<u8>,
}

/// [MS-TSGU] 2.2.5.4.1 `UdpPktType`.
#[repr(u16)]
#[derive(Eq, PartialEq, Copy, Clone, Debug)]
pub enum UdpPktType {
    ConnectReq = 1,
    ConnectResp = 2,
    Payload = 3,
    Disconnect = 4,
    ConnectReqFragment = 5,
}

impl UdpPktType {
    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn as_u16(self) -> u16 {
        self as u16
    }
}

impl TryFrom<u16> for UdpPktType {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Ok(match value {
            1 => Self::ConnectReq,
            2 => Self::ConnectResp,
            3 => Self::Payload,
            4 => Self::Disconnect,
            5 => Self::ConnectReqFragment,
            _ => return Err(()),
        })
    }
}

/// [MS-TSGU] 2.2.11.7 `UDP_PACKET_HEADER`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UdpPacketHeader {
    pub pkt_id: u16,
    /// Length of the body **excluding** this header.
    pub pkt_len: u16,
}

impl UdpPacketHeader {
    pub(crate) const FIXED_PART_SIZE: usize = 2 /* pkt_id */ + 2 /* pkt_len */;
}

impl Encode for UdpPacketHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.pkt_id);
        dst.write_u16(self.pkt_len);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "UDP_PACKET_HEADER"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for UdpPacketHeader {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self {
            pkt_id: src.read_u16(),
            pkt_len: src.read_u16(),
        })
    }
}

/// [MS-TSGU] 2.2.11.1 `AASYNDATA`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AaSynData {
    pub up_stream_mtu: u16,
    pub down_stream_mtu: u16,
    pub lossy: u32,
    pub send_isn: i32,
}

impl AaSynData {
    pub(crate) const FIXED_PART_SIZE: usize =
        2 /* up */ + 2 /* down */ + 4 /* lossy */ + 4 /* send_isn */;
}

impl Encode for AaSynData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.up_stream_mtu);
        dst.write_u16(self.down_stream_mtu);
        dst.write_u32(self.lossy);
        dst.write_i32(self.send_isn);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "AASYNDATA"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for AaSynData {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self {
            up_stream_mtu: src.read_u16(),
            down_stream_mtu: src.read_u16(),
            lossy: src.read_u32(),
            send_isn: src.read_i32(),
        })
    }
}

/// [MS-TSGU] 2.2.11.2 `AASYNDATARESP`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AaSynDataResp {
    pub up_stream_mtu: u16,
    pub down_stream_mtu: u16,
    pub recv_isn: i32,
}

impl AaSynDataResp {
    pub(crate) const FIXED_PART_SIZE: usize = 2 /* up */ + 2 /* down */ + 4 /* recv_isn */;
}

impl Decode<'_> for AaSynDataResp {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self {
            up_stream_mtu: src.read_u16(),
            down_stream_mtu: src.read_u16(),
            recv_isn: src.read_i32(),
        })
    }
}

/// [MS-TSGU] 2.2.11.3 `CONNECT_PKT` (body after DTLS, not including outer DTLS records).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectPkt {
    pub target_port: u16,
    pub syn_data: AaSynData,
    pub authn_cookie: Vec<u8>,
}

impl ConnectPkt {
    /// Build a connect request from a main-channel UDP offer and target resource port.
    pub fn from_offer(offer: &GwUdpOffer, target_port: u16, syn_data: AaSynData) -> Self {
        Self {
            target_port,
            syn_data,
            authn_cookie: offer.authn_cookie.clone(),
        }
    }
}

impl Encode for ConnectPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        let body_len = self.size() - UdpPacketHeader::FIXED_PART_SIZE;
        let hdr = UdpPacketHeader {
            pkt_id: UdpPktType::ConnectReq.as_u16(),
            pkt_len: cast_int!("connect pkt body length", body_len)?,
        };
        hdr.encode(dst)?;
        dst.write_u16(self.target_port);
        let cookie_len: u16 = cast_int!("authn cookie length", self.authn_cookie.len())?;
        dst.write_u16(cookie_len);
        self.syn_data.encode(dst)?;
        dst.write_slice(&self.authn_cookie);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "CONNECT_PKT"
    }

    fn size(&self) -> usize {
        UdpPacketHeader::FIXED_PART_SIZE
            + 2 /* target_port */
            + 2 /* cookie_len */
            + AaSynData::FIXED_PART_SIZE
            + self.authn_cookie.len()
    }
}

impl Decode<'_> for ConnectPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        let hdr = UdpPacketHeader::decode(src)?;
        if hdr.pkt_id != UdpPktType::ConnectReq.as_u16() {
            return Err(unsupported_value_err(
                "CONNECT_PKT::pkt_id",
                "pkt_id",
                format!("0x{:x}", hdr.pkt_id),
            ));
        }
        ensure_size!(in: src, size: 4);
        let target_port = src.read_u16();
        let cookie_len = usize::from(src.read_u16());
        let syn_data = AaSynData::decode(src)?;
        ensure_size!(in: src, size: cookie_len);
        let authn_cookie = src.read_slice(cookie_len).to_vec();
        Ok(Self {
            target_port,
            syn_data,
            authn_cookie,
        })
    }
}

/// [MS-TSGU] 2.2.11.4 `CONNECT_PKT_RESP`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectPktResp {
    pub syn_response: AaSynDataResp,
    /// Connection result (HRESULT-style `i64` on the wire as 8 little-endian bytes).
    pub result: i64,
}

impl Decode<'_> for ConnectPktResp {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        let hdr = UdpPacketHeader::decode(src)?;
        if hdr.pkt_id != UdpPktType::ConnectResp.as_u16() {
            return Err(unsupported_value_err(
                "CONNECT_PKT_RESP::pkt_id",
                "pkt_id",
                format!("0x{:x}", hdr.pkt_id),
            ));
        }
        let syn_response = AaSynDataResp::decode(src)?;
        ensure_size!(in: src, size: 8);
        let result = src.read_i64();
        Ok(Self { syn_response, result })
    }
}

/// [MS-TSGU] 2.2.11.9 `UDP_CORRELATION_INFO` appended to DTLS ClientHello.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UdpCorrelationInfo {
    pub correlation_id: [u8; 16],
}

impl UdpCorrelationInfo {
    pub const SIZE: usize = 4 /* reserved */ + 2 /* sig1 */ + 16 /* id */ + 2 /* sig2 */ + 2 /* cb */;
    const SIGNATURE1: u16 = 0x1DAA;
    const SIGNATURE2: u16 = 0xAA1D;

    pub fn new(correlation_id: [u8; 16]) -> Self {
        Self { correlation_id }
    }
}

impl Encode for UdpCorrelationInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(0);
        dst.write_u16(Self::SIGNATURE1);
        dst.write_slice(&self.correlation_id);
        dst.write_u16(Self::SIGNATURE2);
        dst.write_u16(cast_int!("udp correlation struct size", Self::SIZE)?);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "UDP_CORRELATION_INFO"
    }

    fn size(&self) -> usize {
        Self::SIZE
    }
}

impl Decode<'_> for UdpCorrelationInfo {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: Self::SIZE);
        let _reserved = src.read_u32();
        let sig1 = src.read_u16();
        if sig1 != Self::SIGNATURE1 {
            return Err(unsupported_value_err(
                "UDP_CORRELATION_INFO::uSignature1",
                "sig1",
                format!("0x{sig1:x}"),
            ));
        }
        let correlation_id = src.read_array();
        let sig2 = src.read_u16();
        if sig2 != Self::SIGNATURE2 {
            return Err(unsupported_value_err(
                "UDP_CORRELATION_INFO::uSignature2",
                "sig2",
                format!("0x{sig2:x}"),
            ));
        }
        let cb = src.read_u16();
        if usize::from(cb) != Self::SIZE {
            return Err(unsupported_value_err(
                "UDP_CORRELATION_INFO::uCbStruct",
                "cb",
                format!("{cb}"),
            ));
        }
        Ok(Self { correlation_id })
    }
}

/// Split a connect request into fragment packets ([MS-TSGU] 2.2.11.10) when needed for MTU.
pub fn fragment_connect_pkt(
    connect: &ConnectPkt,
    max_fragment_payload: usize,
) -> ironrdp_core::EncodeResult<Vec<Vec<u8>>> {
    if max_fragment_payload == 0 {
        return Err(other_err("udp fragment", "max fragment payload must be non-zero"));
    }

    let mut full = vec![0u8; connect.size()];
    {
        let mut cur = WriteCursor::new(&mut full);
        connect.encode(&mut cur)?;
    }
    // Fragments carry CONNECT_PKT bytes after the UDP header of the full packet.
    let payload = &full[UdpPacketHeader::FIXED_PART_SIZE..];
    let total = payload.len().div_ceil(max_fragment_payload);
    let total_u16: u16 = cast_int!("udp fragment count", total)?;

    let mut out = Vec::with_capacity(total);
    for (idx, chunk) in payload.chunks(max_fragment_payload).enumerate() {
        let frag_id: u16 = cast_int!("udp fragment id", idx)?;
        let chunk_len: u16 = cast_int!("udp fragment length", chunk.len())?;
        // header(4) + frag_id(2) + no_of_fragments(2) + cb(2) + chunk
        let body_len = 2 + 2 + 2 + chunk.len();
        let mut buf = vec![0u8; UdpPacketHeader::FIXED_PART_SIZE + body_len];
        {
            let mut cur = WriteCursor::new(&mut buf);
            UdpPacketHeader {
                pkt_id: UdpPktType::ConnectReqFragment.as_u16(),
                pkt_len: cast_int!("udp fragment body length", body_len)?,
            }
            .encode(&mut cur)?;
            cur.write_u16(frag_id);
            cur.write_u16(total_u16);
            cur.write_u16(chunk_len);
            cur.write_slice(chunk);
        }
        out.push(buf);
    }
    Ok(out)
}

/// Encode a complete CONNECT_PKT for a channel UDP offer.
pub fn encode_connect_request(
    offer: &GwUdpOffer,
    target_port: u16,
    syn_data: AaSynData,
) -> ironrdp_core::EncodeResult<Vec<u8>> {
    let pkt = ConnectPkt::from_offer(offer, target_port, syn_data);
    let mut buf = vec![0u8; pkt.size()];
    let mut cur = WriteCursor::new(&mut buf);
    pkt.encode(&mut cur)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(&bytes[..2], &1u16.to_le_bytes()); // ConnectReq
        let mut cur = ReadCursor::new(&bytes);
        let decoded = ConnectPkt::decode(&mut cur).expect("decode");
        assert_eq!(decoded.target_port, 3389);
        assert_eq!(decoded.syn_data, syn);
        assert_eq!(decoded.authn_cookie, offer.authn_cookie);
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
    fn fragment_connect_pkt_covers_payload() {
        let offer = GwUdpOffer {
            port: 3391,
            authn_cookie: vec![7; 40],
        };
        let pkt = ConnectPkt::from_offer(&offer, 2179, AaSynData::default());
        let frags = fragment_connect_pkt(&pkt, 16).expect("fragment");
        assert!(frags.len() > 1);
        let mut reassembled = Vec::new();
        for frag in &frags {
            let mut cur = ReadCursor::new(frag);
            let hdr = UdpPacketHeader::decode(&mut cur).unwrap();
            assert_eq!(hdr.pkt_id, UdpPktType::ConnectReqFragment.as_u16());
            let _id = cur.read_u16();
            let total = cur.read_u16();
            assert_eq!(usize::from(total), frags.len());
            let len = usize::from(cur.read_u16());
            reassembled.extend_from_slice(cur.read_slice(len));
        }
        let mut full = encode_to_vec(&pkt);
        assert_eq!(reassembled, full.split_off(UdpPacketHeader::FIXED_PART_SIZE));
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
}
