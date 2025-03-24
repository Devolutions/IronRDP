use ironrdp_core::{unsupported_value_err, Decode, Encode, ReadCursor, WriteCursor};

/// 2.2.5.3.3 HTTP_PACKET_TYPE Enumeration
#[repr(u16)]
#[derive(Eq, PartialEq, Copy, Clone, Debug)]
pub enum PktTy {
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
}

impl Default for PktTy {
    fn default() -> Self {
        PktTy::Invalid
    }
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
            0x0a => PktTy::Data,
            0x10 => PktTy::ChannelClose,
            _ => return Err(()),
        };
        return Ok(mapped)
    }
}

/// 2.2.10.9 HTTP_PACKET_HEADER Structure
#[derive(Default, Debug)]
pub struct PktHdr {
    pub ty: PktTy,
    _reserved: u16,
    pub length: u32,
}

impl Encode for PktHdr {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        dst.write_u16(self.ty as u16);
        dst.write_u16(self._reserved);
        dst.write_u32(self.length);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_PACKET_HEADER"
    }

    fn size(&self) -> usize {
        8
    }
}

impl<'a> Decode<'a> for PktHdr {
    fn decode(src: &mut ReadCursor<'a>) -> ironrdp_core::DecodeResult<Self> {
        let ty = src.read_u16();
        let mty = PktTy::try_from(ty).map_err(|_| unsupported_value_err("PktHdr::ty", "ty", format!("0x{:x}", ty)))?;

        Ok(PktHdr {
            ty: mty,
            _reserved: src.read_u16(),
            length: src.read_u32()
        })
    }
}

/// 2.2.10.10 HTTP_HANDSHAKE_REQUEST_PACKET Structure
#[derive(Default)]
pub struct HandshakeReqPkt {
    pub ver_major: u8,
    pub ver_minor: u8,
    pub client_version: u16,
    pub extended_auth: u16,
}

impl Encode for HandshakeReqPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        let hdr = PktHdr {
            ty: PktTy::HandshakeReq,
            length: self.size() as u32,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u8(self.ver_major);
        dst.write_u8(self.ver_minor);
        dst.write_u16(self.client_version);
        dst.write_u16(self.extended_auth);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_HANDSHAKE_REQUEST_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::default().size() + 6
    }
}

/// 2.2.10.11 HTTP_HANDSHAKE_RESPONSE_PACKET Structure
#[derive(Debug)]
pub struct HandshakeRespPkt {
    pub error_code: u32,
    pub ver_major: u8,
    pub ver_minor: u8,
    pub server_version: u16,
    pub extended_auth: u16,
}

impl Decode<'_> for HandshakeRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        Ok(HandshakeRespPkt {
            error_code: src.read_u32(),
            ver_major: src.read_u8(),
            ver_minor: src.read_u8(),
            server_version: src.read_u16(),
            extended_auth: src.read_u16(),
        })
    }
}

/// 2.2.10.18 HTTP_TUNNEL_PACKET 
#[derive(Default)]
pub struct TunnelReqPkt {
    pub caps: u32,
    pub fields_present: u16,
    pub(crate) _reserved: u16,
}

impl Encode for TunnelReqPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        let hdr = PktHdr {
            ty: PktTy::TunnelCreate,
            length: self.size() as u32,
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
enum HttpCapsTy {
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
    SOH = 4,
    Consent = 0x10
}

/// 2.2.10.20 HTTP_TUNNEL_RESPONSE Structure
#[derive(Debug, Default)]
pub struct TunnelRespPkt {
    pub server_version: u16,
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

impl Decode<'_> for TunnelRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        let mut pkt = TunnelRespPkt {
            server_version: src.read_u16(),
            status_code: src.read_u32(),
            fields_present: src.read_u16(),
            _reserved: src.read_u16(),
            ..TunnelRespPkt::default()
        };

        if pkt.fields_present & (HttpTunnelResponseFields::TunnelID as u16) != 0 {
            pkt.tunnel_id = Some(src.read_u32());
        }
        if pkt.fields_present & (HttpTunnelResponseFields::Caps as u16) != 0 {
            pkt.caps_flags = Some(src.read_u32());
        }
        if pkt.fields_present & (HttpTunnelResponseFields::SOH as u16) != 0 {
            pkt.nonce = Some(src.read_u16());
            let len = src.read_u16();
            pkt.server_cert = src.read_slice(len as usize).to_vec();
        }
        if pkt.fields_present & (HttpTunnelResponseFields::Consent as u16) != 0 {
            let len = src.read_u16();
            pkt.consent_msg = src.read_slice(len as usize).to_vec();
        }

        Ok(pkt)
    }
}

/// 2.2.10.7 HTTP_EXTENDED_AUTH_PACKET Structure
pub struct ExtendedAuthPkt {
    error_code: u32,
    blob: Vec<u8>
}

impl Encode for ExtendedAuthPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        let hdr = PktHdr {
            ty: PktTy::ExtendedAuth,
            length: self.size() as u32,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u32(self.error_code);
        dst.write_u16(self.blob.len() as u16);
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
        let error_code = src.read_u32();
        let len = src.read_u16();

        Ok(ExtendedAuthPkt {
            error_code,
            blob: src.read_slice(len as usize).to_vec(),
        })
    }
}

/// 2.2.10.14 HTTP_TUNNEL_AUTH_PACKET Structure
pub struct TunnelAuthPkt {
    pub fields_present: u16,
    pub client_name: String,
}

impl Encode for TunnelAuthPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        let hdr = PktHdr {
            ty: PktTy::TunnelAuth,
            length: self.size() as u32,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u16(self.fields_present);
        dst.write_u16(2*(self.client_name.len() as u16+1));
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
        PktHdr::default().size() + 4 + 2*(self.client_name.len()+1)
    }
}

/// 2.2.10.16 HTTP_TUNNEL_AUTH_RESPONSE Structure
#[derive(Debug)]
pub struct TunnelAuthRespPkt {
    pub error_code: u32,
    fields_present: u16,
    _reserved: u16,
}

impl Decode<'_> for TunnelAuthRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        Ok(TunnelAuthRespPkt {
            error_code: src.read_u32(),
            fields_present: src.read_u16(),
            _reserved: src.read_u16(),
        })
    }
}

/// 2.2.10.2 HTTP_CHANNEL_PACKET 
pub struct ChannelPkt {
    pub resources: Vec<String>,
    pub port: u16,
    pub protocol: u16,
}

impl Encode for ChannelPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        let hdr = PktHdr {
            ty: PktTy::ChannelCreate,
            length: self.size() as u32,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u8(self.resources.len() as u8);
        dst.write_u8(0); // alt_names
        dst.write_u16(self.port);
        dst.write_u16(self.protocol);

        // 2.2.10.3 HTTP_CHANNEL_PACKET_VARIABLE
        for res in &self.resources {
            dst.write_u16(2*(res.len()+1) as u16);
            // dst.write_slice(res.as_bytes());
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
        PktHdr::default().size() + 6 + self.resources.iter().map(|x| 2+2*(x.len()+1)).sum::<usize>()
    }
}

/// 2.2.10.4 HTTP_CHANNEL_RESPONSE
#[derive(Default, Debug)]
pub struct ChannelResp {
    pub error_code: u32,
    fields_present: u16,
    _reserved: u16,

    /// 2.2.10.5 HTTP_CHANNEL_RESPONSE_OPTIONAL
    chan_id: Option<u32>,
    udp_port: u16,
    authn_cookie: Vec<u8>,
}

impl Decode<'_> for ChannelResp {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        let mut resp = ChannelResp {
            error_code: src.read_u32(), 
            fields_present: src.read_u16(), 
            _reserved: src.read_u16(),
            ..ChannelResp::default()
        };
        if resp.fields_present & 1 != 0 {
            resp.chan_id = Some(src.read_u32());
        }
        if resp.fields_present & 2 != 0 {
            resp.udp_port = src.read_u16();
        }
        if resp.fields_present & 4 != 0 {
            let len = src.read_u16();
            resp.authn_cookie = src.read_slice(len as usize).to_vec();
        }
        Ok(resp)
    }
}

/// 2.2.10.6 HTTP_DATA_PACKET 
pub struct DataPkt<'a> {
    pub data: &'a [u8]
}

impl Encode for DataPkt<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        let hdr = PktHdr {
            ty: PktTy::Data,
            length: self.size() as u32,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;
        dst.write_u16(self.data.len() as u16);
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
        let len = src.read_u16();
        Ok(DataPkt{ data: src.read_slice(len as usize) })
    }
}