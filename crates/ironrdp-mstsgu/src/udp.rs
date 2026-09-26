//! RDG-UDP packet layouts ([MS-TSGU] 2.2.5.4 / 2.2.11).
//!
//! Opening a live side channel still requires DTLS + MS-RDPEUDP.
//! This module only encodes and decodes the MS-TSGU UDP framing used after the main HTTP channel supplies a cookie.
//!
//! [MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188

use ironrdp_core::{
    Decode, Encode, ReadCursor, WriteCursor, cast_int, ensure_fixed_part_size, ensure_size, unsupported_value_err,
};

/// Maximum CONNECT_PKT fragment payload ([MS-TSGU] 3.8.3 `MAX_CONNECT_REQ_FRAGMENT_SIZE`).
///
/// [MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
pub const MAX_CONNECT_REQ_FRAGMENT_SIZE: usize = 1000;

/// Parameters from [`HTTP_CHANNEL_RESPONSE`][channel-resp] that enable a future RDG-UDP side channel.
///
/// [channel-resp]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GwUdpOffer {
    /// UDP port on the gateway that accepts the side channel.
    pub port: u16,
    /// Opaque authentication cookie for [`CONNECT_PKT`](ConnectPkt).
    pub authn_cookie: Vec<u8>,
}

/// [2.2.5.4.1] `UdpPktType` enumeration.
///
/// [2.2.5.4.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
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
    pub fn as_u16(self) -> u16 {
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

/// [2.2.11.7] `UDP_PACKET_HEADER` structure.
///
/// [2.2.11.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UdpPacketHeader {
    pub pkt_id: u16,
    /// Length of the body **excluding** this header.
    pub pkt_len: u16,
}

impl UdpPacketHeader {
    pub const FIXED_PART_SIZE: usize = 2 /* pktID */ + 2 /* pktLen */;
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

/// [2.2.11.1] `AASYNDATA` structure.
///
/// [2.2.11.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AaSynData {
    pub up_stream_mtu: u16,
    pub down_stream_mtu: u16,
    pub lossy: u32,
    pub send_isn: i32,
}

impl AaSynData {
    pub const FIXED_PART_SIZE: usize =
        2 /* uUpStreamMtu */ + 2 /* uDownStreamMtu */ + 4 /* fLossy */ + 4 /* snSendISN */;
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

/// [2.2.11.2] `AASYNDATARESP` structure.
///
/// [2.2.11.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AaSynDataResp {
    pub up_stream_mtu: u16,
    pub down_stream_mtu: u16,
    pub recv_isn: i32,
}

impl AaSynDataResp {
    pub const FIXED_PART_SIZE: usize = 2 /* uUpStreamMtu */ + 2 /* uDownStreamMtu */ + 4 /* snRecvISN */;
}

impl Encode for AaSynDataResp {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.up_stream_mtu);
        dst.write_u16(self.down_stream_mtu);
        dst.write_i32(self.recv_isn);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "AASYNDATARESP"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
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

/// [2.2.11.3] `CONNECT_PKT` structure (body after DTLS, not including outer DTLS records).
///
/// [2.2.11.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectPkt {
    pub target_port: u16,
    pub syn_data: AaSynData,
    pub authn_cookie: Vec<u8>,
}

impl ConnectPkt {
    const FIXED_BODY_SIZE: usize = 2 /* usPortNumber */ + 2 /* cbAuthnCookieLen */ + AaSynData::FIXED_PART_SIZE;

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
        UdpPacketHeader::FIXED_PART_SIZE + Self::FIXED_BODY_SIZE + self.authn_cookie.len()
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
                None,
            ));
        }
        let body_len = usize::from(hdr.pkt_len);
        ensure_size!(in: src, size: body_len);
        if body_len < Self::FIXED_BODY_SIZE {
            return Err(unsupported_value_err(
                "CONNECT_PKT::pkt_len",
                "pkt_len",
                format!("{body_len}"),
                None,
            ));
        }
        let target_port = src.read_u16();
        let cookie_len = usize::from(src.read_u16());
        if body_len != Self::FIXED_BODY_SIZE + cookie_len {
            return Err(unsupported_value_err(
                "CONNECT_PKT::pkt_len",
                "pkt_len",
                format!("{body_len}"),
                None,
            ));
        }
        let syn_data = AaSynData::decode(src)?;
        let authn_cookie = src.read_slice(cookie_len).to_vec();
        Ok(Self {
            target_port,
            syn_data,
            authn_cookie,
        })
    }
}

/// [2.2.11.4] `CONNECT_PKT_RESP` structure.
///
/// [2.2.11.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectPktResp {
    pub syn_response: AaSynDataResp,
    /// Connection result (HRESULT-style `i64` on the wire as 8 little-endian bytes).
    pub result: i64,
}

impl ConnectPktResp {
    const BODY_SIZE: usize = AaSynDataResp::FIXED_PART_SIZE + 8 /* result */;
}

impl Encode for ConnectPktResp {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        let body_len = self.size() - UdpPacketHeader::FIXED_PART_SIZE;
        let hdr = UdpPacketHeader {
            pkt_id: UdpPktType::ConnectResp.as_u16(),
            pkt_len: cast_int!("connect resp body length", body_len)?,
        };
        hdr.encode(dst)?;
        self.syn_response.encode(dst)?;
        dst.write_i64(self.result);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "CONNECT_PKT_RESP"
    }

    fn size(&self) -> usize {
        UdpPacketHeader::FIXED_PART_SIZE + Self::BODY_SIZE
    }
}

impl Decode<'_> for ConnectPktResp {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        let hdr = UdpPacketHeader::decode(src)?;
        if hdr.pkt_id != UdpPktType::ConnectResp.as_u16() {
            return Err(unsupported_value_err(
                "CONNECT_PKT_RESP::pkt_id",
                "pkt_id",
                format!("0x{:x}", hdr.pkt_id),
                None,
            ));
        }
        if usize::from(hdr.pkt_len) != Self::BODY_SIZE {
            return Err(unsupported_value_err(
                "CONNECT_PKT_RESP::pkt_len",
                "pkt_len",
                format!("{}", hdr.pkt_len),
                None,
            ));
        }
        let syn_response = AaSynDataResp::decode(src)?;
        ensure_size!(in: src, size: 8);
        let result = src.read_i64();
        Ok(Self { syn_response, result })
    }
}

/// [2.2.11.5] `DATA_PKT` structure.
///
/// [2.2.11.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataPkt {
    pub data: Vec<u8>,
}

impl Encode for DataPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        let hdr = UdpPacketHeader {
            pkt_id: UdpPktType::Payload.as_u16(),
            pkt_len: cast_int!("data pkt body length", self.data.len())?,
        };
        hdr.encode(dst)?;
        dst.write_slice(&self.data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DATA_PKT"
    }

    fn size(&self) -> usize {
        UdpPacketHeader::FIXED_PART_SIZE + self.data.len()
    }
}

impl Decode<'_> for DataPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        let hdr = UdpPacketHeader::decode(src)?;
        if hdr.pkt_id != UdpPktType::Payload.as_u16() {
            return Err(unsupported_value_err(
                "DATA_PKT::pkt_id",
                "pkt_id",
                format!("0x{:x}", hdr.pkt_id),
                None,
            ));
        }
        let data_len = usize::from(hdr.pkt_len);
        ensure_size!(in: src, size: data_len);
        Ok(Self {
            data: src.read_slice(data_len).to_vec(),
        })
    }
}

/// [2.2.11.6] `DISC_PKT` structure.
///
/// [2.2.11.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiscPkt {
    /// Disconnect reason (HRESULT-style `i64` on the wire as 8 little-endian bytes).
    pub reason: i64,
}

impl DiscPkt {
    const BODY_SIZE: usize = 8 /* discReason */;
}

impl Encode for DiscPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        let hdr = UdpPacketHeader {
            pkt_id: UdpPktType::Disconnect.as_u16(),
            pkt_len: cast_int!("disconnect pkt body length", Self::BODY_SIZE)?,
        };
        hdr.encode(dst)?;
        dst.write_i64(self.reason);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DISC_PKT"
    }

    fn size(&self) -> usize {
        UdpPacketHeader::FIXED_PART_SIZE + Self::BODY_SIZE
    }
}

impl Decode<'_> for DiscPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        let hdr = UdpPacketHeader::decode(src)?;
        if hdr.pkt_id != UdpPktType::Disconnect.as_u16() {
            return Err(unsupported_value_err(
                "DISC_PKT::pkt_id",
                "pkt_id",
                format!("0x{:x}", hdr.pkt_id),
                None,
            ));
        }
        if usize::from(hdr.pkt_len) != Self::BODY_SIZE {
            return Err(unsupported_value_err(
                "DISC_PKT::pkt_len",
                "pkt_len",
                format!("{}", hdr.pkt_len),
                None,
            ));
        }
        ensure_size!(in: src, size: Self::BODY_SIZE);
        Ok(Self { reason: src.read_i64() })
    }
}

/// [2.2.11.9] `UDP_CORRELATION_INFO` structure appended to DTLS ClientHello.
///
/// This type encodes and decodes the trailing blob only.
/// It does not inject the structure into a DTLS handshake.
///
/// [2.2.11.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UdpCorrelationInfo {
    pub correlation_id: [u8; 16],
}

impl UdpCorrelationInfo {
    pub const SIZE: usize = 4 /* uReserved */ + 2 /* uSignature1 */ + 16 /* uCorrelationId */ + 2 /* uSignature2 */ + 2 /* uCbStruct */;
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
                None,
            ));
        }
        let correlation_id = src.read_array();
        let sig2 = src.read_u16();
        if sig2 != Self::SIGNATURE2 {
            return Err(unsupported_value_err(
                "UDP_CORRELATION_INFO::uSignature2",
                "sig2",
                format!("0x{sig2:x}"),
                None,
            ));
        }
        let cb = src.read_u16();
        if usize::from(cb) != Self::SIZE {
            return Err(unsupported_value_err(
                "UDP_CORRELATION_INFO::uCbStruct",
                "cb",
                format!("{cb}"),
                None,
            ));
        }
        Ok(Self { correlation_id })
    }
}

/// Split a complete `CONNECT_PKT` into fragment packets ([2.2.11.10] / [3.8.3]).
///
/// [MS-TSGU] 3.8.3 sets `reqLen` to `hdr.pktLen + sizeof(UDP_PACKET_HEADER)` and splits that
/// full request buffer into `MAX_CONNECT_REQ_FRAGMENT_SIZE` (1000-byte) payloads.
///
/// [2.2.11.10]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
/// [3.8.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
pub fn fragment_connect_pkt(connect: &ConnectPkt) -> ironrdp_core::EncodeResult<Vec<Vec<u8>>> {
    let mut full = vec![0u8; connect.size()];
    {
        let mut cur = WriteCursor::new(&mut full);
        connect.encode(&mut cur)?;
    }
    let total = full.len().div_ceil(MAX_CONNECT_REQ_FRAGMENT_SIZE);
    let total_u16: u16 = cast_int!("udp fragment count", total)?;

    let mut out = Vec::with_capacity(total);
    for (idx, chunk) in full.chunks(MAX_CONNECT_REQ_FRAGMENT_SIZE).enumerate() {
        let frag_id: u16 = cast_int!("udp fragment id", idx)?;
        let chunk_len: u16 = cast_int!("udp fragment length", chunk.len())?;
        let body_len = 2 /* usFragmentID */ + 2 /* usNoOfFragments */ + 2 /* cbFragmentLength */ + chunk.len();
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

/// Encode a complete `CONNECT_PKT` for a channel UDP offer.
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
