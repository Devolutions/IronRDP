use bitflags::bitflags;
use ironrdp_core::{
    cast_int, cast_length, ensure_fixed_part_size, ensure_size, unsupported_value_err, Decode, Encode, ReadCursor,
    WriteCursor,
};

bitflags! {
    /// 2.2.5.3.2 HTTP_EXTENDED_AUTH Enumeration
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) struct HttpExtendedAuth: u16 {
        const HTTP_EXTENDED_AUTH_NONE = 0x01;
        const HTTP_EXTENDED_AUTH_SC = 0x01;
        const HTTP_EXTENDED_AUTH_PAA = 0x02;
        const HTTP_EXTENDED_AUTH_SSPI_NTLM = 0x04;
    }
}

/// 2.2.5.3.3 HTTP_PACKET_TYPE Enumeration
#[repr(u16)]
#[derive(Eq, PartialEq, Copy, Clone, Debug, Default)]
pub(crate) enum PktTy {
    #[default]
    Invalid,
    HandshakeReq = 0x01,
    HandshakeResp = 0x02,
    ExtendedAuth = 0x03,
    TunnelCreate = 0x04,
    TunnelResp = 0x05,
    TunnelAuth = 0x06,
    TunnelAuthResponse = 0x07,
    ChannelCreate = 0x08,
    ChannelResp = 0x09,
    ChannelClose = 0x10,
    Data = 0x0A,
    ServiceMessage = 0x0B,
    ReauthMessage = 0x0C,
    Keepalive = 0x0D,
}

impl TryFrom<u16> for PktTy {
    type Error = ();

    fn try_from(val: u16) -> Result<Self, Self::Error> {
        let mapped = match val {
            0x01 => PktTy::HandshakeReq,
            0x02 => PktTy::HandshakeResp,
            0x03 => PktTy::ExtendedAuth,
            0x04 => PktTy::TunnelCreate,
            0x05 => PktTy::TunnelResp,
            0x06 => PktTy::TunnelAuth,
            0x07 => PktTy::TunnelAuthResponse,
            0x08 => PktTy::ChannelCreate,
            0x09 => PktTy::ChannelResp,
            0x0A => PktTy::Data,
            0x0B => PktTy::ServiceMessage,
            0x0C => PktTy::ReauthMessage,
            0x0D => PktTy::Keepalive,
            0x10 => PktTy::ChannelClose,
            _ => return Err(()),
        };
        Ok(mapped)
    }
}

/// 2.2.10.9 HTTP_PACKET_HEADER Structure
#[derive(Default, Debug)]
pub(crate) struct PktHdr {
    pub ty: PktTy,
    pub _reserved: u16,
    pub length: u32,
}

impl PktHdr {
    const FIXED_PART_SIZE: usize = 4 /* ty */ + 2 /* _reserved */ + 2 /* length */;
}

impl Encode for PktHdr {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.ty as u16);
        dst.write_u16(self._reserved);
        dst.write_u32(self.length);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_PACKET_HEADER"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'a> Decode<'a> for PktHdr {
    fn decode(src: &mut ReadCursor<'a>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let ty = src.read_u16();
        let mty = PktTy::try_from(ty).map_err(|_| unsupported_value_err("PktHdr::ty", "ty", format!("0x{ty:x}")))?;

        Ok(PktHdr {
            ty: mty,
            _reserved: src.read_u16(),
            length: src.read_u32(),
        })
    }
}

/// 2.2.10.10 HTTP_HANDSHAKE_REQUEST_PACKET Structure
#[derive(Default)]
pub(crate) struct HandshakeReqPkt {
    pub ver_major: u8,
    pub ver_minor: u8,
    pub client_version: u16,
    pub extended_auth: HttpExtendedAuth,
}

impl Encode for HandshakeReqPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::HandshakeReq,
            length: u32::try_from(self.size()).expect("handshake packet size fits in u32"),
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u8(self.ver_major);
        dst.write_u8(self.ver_minor);
        dst.write_u16(self.client_version);
        dst.write_u16(self.extended_auth.bits());

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_HANDSHAKE_REQUEST_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE + 6
    }
}

/// 2.2.10.11 HTTP_HANDSHAKE_RESPONSE_PACKET Structure
#[derive(Debug)]
pub(crate) struct HandshakeRespPkt {
    pub error_code: u32,
    pub ver_major: u8,
    pub ver_minor: u8,
    pub server_version: u16,
    pub _extended_auth: HttpExtendedAuth,
}

impl HandshakeRespPkt {
    const FIXED_PART_SIZE: usize = 4 /* error_code */ + 1 /* ver_major */ + 1 /* ver_minor */ + 2 /* server_auth */ + 1 /*extended_auth*/;
}

impl Decode<'_> for HandshakeRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        Ok(HandshakeRespPkt {
            error_code: src.read_u32(),
            ver_major: src.read_u8(),
            ver_minor: src.read_u8(),
            server_version: src.read_u16(),
            _extended_auth: {
                let raw = src.read_u16();
                HttpExtendedAuth::from_bits(raw)
                    .ok_or_else(|| unsupported_value_err("HandshakeResp", "extended_auth", format!("0x{raw:x}")))?
            },
        })
    }
}

/// 2.2.10.18 HTTP_TUNNEL_PACKET
#[derive(Default)]
pub(crate) struct TunnelReqPkt {
    pub caps: u32,
    pub fields_present: u16,
    pub _reserved: u16,
}

impl Encode for TunnelReqPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::TunnelCreate,
            length: u32::try_from(self.size()).expect("tunnel request packet size fits in u32"),
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u32(self.caps);
        dst.write_u16(self.fields_present);
        dst.write_u16(self._reserved);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_TUNNEL_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::default().size() + 8
    }
}

/// 2.2.5.3.9 HTTP_CAPABILITY_TYPE Enumeration
#[repr(u32)]
#[expect(dead_code)]
pub(crate) enum HttpCapsTy {
    QuarSOH = 1,
    IdleTimeout = 2,
    MessagingConsentSign = 4,
    MessagingServiceMsg = 8,
    Reauth = 0x10,
    UdpTransport = 0x20,
}

/// 2.2.5.3.8 HTTP_TUNNEL_RESPONSE_FIELDS_PRESENT_FLAGS
#[repr(u16)]
enum HttpTunnelResponseFields {
    TunnelID = 1,
    Caps = 2,
    /// nonce & server_cert
    Soh = 4,
    Consent = 0x10,
}

/// 2.2.10.20 HTTP_TUNNEL_RESPONSE Structure
#[derive(Debug, Default)]
pub(crate) struct TunnelRespPkt {
    pub _server_version: u16,
    pub status_code: u32,
    pub fields_present: u16,
    pub _reserved: u16,

    // 2.2.10.21 HTTP_TUNNEL_RESPONSE_OPTIONAL
    pub tunnel_id: Option<u32>,
    pub caps_flags: Option<u32>,
    pub nonce: Option<u16>,
    pub server_cert: Vec<u8>,
    pub consent_msg: Vec<u8>,
}

impl TunnelRespPkt {
    const FIXED_PART_SIZE: usize = 2 /* server_version */ + 4 /* status_code */ + 2 /* fields_present */ + 2 /* reserved */;
}

impl Decode<'_> for TunnelRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let mut pkt = TunnelRespPkt {
            _server_version: src.read_u16(),
            status_code: src.read_u32(),
            fields_present: src.read_u16(),
            _reserved: src.read_u16(),
            ..TunnelRespPkt::default()
        };

        if pkt.fields_present & (HttpTunnelResponseFields::TunnelID as u16) != 0 {
            ensure_size!(in: src, size: 4);
            pkt.tunnel_id = Some(src.read_u32());
        }
        if pkt.fields_present & (HttpTunnelResponseFields::Caps as u16) != 0 {
            ensure_size!(in: src, size: 4);
            pkt.caps_flags = Some(src.read_u32());
        }
        if pkt.fields_present & (HttpTunnelResponseFields::Soh as u16) != 0 {
            ensure_size!(in: src, size: 2 + 2);
            pkt.nonce = Some(src.read_u16());
            let len = src.read_u16();
            ensure_size!(in: src, size: len as usize);
            pkt.server_cert = src.read_slice(len as usize).to_vec();
        }
        if pkt.fields_present & (HttpTunnelResponseFields::Consent as u16) != 0 {
            ensure_size!(in: src, size: 2);
            let len = src.read_u16();
            ensure_size!(in: src, size: len as usize);
            pkt.consent_msg = src.read_slice(len as usize).to_vec();
        }

        Ok(pkt)
    }
}

/// 2.2.10.7 HTTP_EXTENDED_AUTH_PACKET Structure
pub(crate) struct ExtendedAuthPkt {
    error_code: u32,
    blob: Vec<u8>,
}

impl Encode for ExtendedAuthPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::ExtendedAuth,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u32(self.error_code);
        let blob_len: u16 = cast_int!("blob length", self.blob.len())?;
        dst.write_u16(blob_len);
        dst.write_slice(&self.blob);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_EXTENDED_AUTH_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::default().size() + 6 + self.blob.len()
    }
}

impl Decode<'_> for ExtendedAuthPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 4 + 2);
        let error_code = src.read_u32();
        let len = src.read_u16();
        ensure_size!(in: src, size: len as usize);

        Ok(ExtendedAuthPkt {
            error_code,
            blob: src.read_slice(len as usize).to_vec(),
        })
    }
}

/// 2.2.10.14 HTTP_TUNNEL_AUTH_PACKET Structure
pub(crate) struct TunnelAuthPkt {
    pub fields_present: u16,
    pub client_name: String,
}

impl Encode for TunnelAuthPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::TunnelAuth,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u16(self.fields_present);

        let client_name_len = self.client_name.encode_utf16().count() * 2 + 2; // Add 2 to account for a null terminator (0x0000).
        let client_name_len: u16 = cast_int!("client name length", client_name_len)?;
        dst.write_u16(client_name_len);

        for c in self.client_name.encode_utf16() {
            dst.write_u16(c);
        }

        dst.write_u16(0);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_TUNNEL_AUTH_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::default().size() + 4 + 2 * (self.client_name.len() + 1)
    }
}

/// 2.2.10.16 HTTP_TUNNEL_AUTH_RESPONSE Structure
#[derive(Debug)]
pub(crate) struct TunnelAuthRespPkt {
    error_code: u32,
    _fields_present: u16,
    _reserved: u16,
}

impl TunnelAuthRespPkt {
    const FIXED_PART_SIZE: usize = 4 /* error_code */ + 2 /* fields_present */ + 2 /* _reserved */;

    pub(crate) fn error_code(&self) -> u32 {
        self.error_code
    }
}

impl Decode<'_> for TunnelAuthRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        Ok(TunnelAuthRespPkt {
            error_code: src.read_u32(),
            _fields_present: src.read_u16(),
            _reserved: src.read_u16(),
        })
    }
}

/// 2.2.10.2 HTTP_CHANNEL_PACKET
pub(crate) struct ChannelPkt {
    pub resources: Vec<String>,
    pub port: u16,
    pub protocol: u16,
}

impl Encode for ChannelPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::ChannelCreate,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        let resources_count: u8 = cast_length!("resources count", self.resources.len())?;
        dst.write_u8(resources_count);
        dst.write_u8(0); // alt_names
        dst.write_u16(self.port);
        dst.write_u16(self.protocol);

        // 2.2.10.3 HTTP_CHANNEL_PACKET_VARIABLE
        for res in &self.resources {
            let res_utf16_len = res.encode_utf16().count() * 2 + 2; // Add 2 to account for a null terminator (0x0000).
            let res_len: u16 = cast_int!("resource name UTF-16 length", res_utf16_len)?;
            dst.write_u16(res_len);
            for b in res.encode_utf16() {
                dst.write_u16(b);
            }
            dst.write_u16(0);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_CHANNEL_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::default().size() + 6 + self.resources.iter().map(|x| 2 + 2 * (x.len() + 1)).sum::<usize>()
    }
}

/// 2.2.10.4 HTTP_CHANNEL_RESPONSE
#[derive(Default, Debug)]
pub(crate) struct ChannelResp {
    error_code: u32,
    fields_present: u16,
    _reserved: u16,

    /// 2.2.10.5 HTTP_CHANNEL_RESPONSE_OPTIONAL
    chan_id: Option<u32>,
    udp_port: u16,
    authn_cookie: Vec<u8>,
}

impl ChannelResp {
    const FIXED_PART_SIZE: usize = 4 /* error_code */ + 2 /* fields_present */ + 2 /* _reserved */;

    pub(crate) fn error_code(&self) -> u32 {
        self.error_code
    }
}

impl Decode<'_> for ChannelResp {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let mut resp = ChannelResp {
            error_code: src.read_u32(),
            fields_present: src.read_u16(),
            _reserved: src.read_u16(),
            ..ChannelResp::default()
        };
        if resp.fields_present & 1 != 0 {
            ensure_size!(in: src, size: 4);
            resp.chan_id = Some(src.read_u32());
        }
        if resp.fields_present & 2 != 0 {
            ensure_size!(in: src, size: 2);
            resp.udp_port = src.read_u16();
        }
        if resp.fields_present & 4 != 0 {
            ensure_size!(in: src, size: 2);
            let len = src.read_u16();
            ensure_size!(in: src, size: len as usize);
            resp.authn_cookie = src.read_slice(len as usize).to_vec();
        }
        Ok(resp)
    }
}

/// 2.2.10.6 HTTP_DATA_PACKET
pub(crate) struct DataPkt<'a> {
    pub data: &'a [u8],
}

impl Encode for DataPkt<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::Data,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;
        let data_len: u16 = cast_int!("data payload length", self.data.len())?;
        dst.write_u16(data_len);
        dst.write_slice(self.data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_DATA_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::default().size() + 2 + self.data.len()
    }
}

impl<'a> Decode<'a> for DataPkt<'a> {
    fn decode(src: &mut ReadCursor<'a>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 2);
        let len = src.read_u16();
        ensure_size!(in: src, size: len as usize);
        Ok(DataPkt {
            data: src.read_slice(len as usize),
        })
    }
}

pub(crate) struct KeepalivePkt;

impl Encode for KeepalivePkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        let hdr = PktHdr {
            ty: PktTy::Keepalive,
            length: u32::try_from(self.size()).expect("keepalive packet size fits in u32"),
            ..PktHdr::default()
        };
        hdr.encode(dst)
    }

    fn name(&self) -> &'static str {
        "KEEPALIVE"
    }

    fn size(&self) -> usize {
        PktHdr::default().size()
    }
}
