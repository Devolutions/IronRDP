use bitflags::bitflags;
use ironrdp_core::{
    Decode, Encode, ReadCursor, WriteCursor, cast_int, cast_length, ensure_fixed_part_size, ensure_size,
    unsupported_value_err,
};

bitflags! {
    /// 2.2.5.3.2 HTTP_EXTENDED_AUTH Enumeration
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) struct HttpExtendedAuth: u16 {
        /// No extended authentication.
        const HTTP_EXTENDED_AUTH_NONE = 0x00;
        /// Smart-card authentication.
        const HTTP_EXTENDED_AUTH_SC = 0x01;
        /// Pluggable authentication and authorization (PAA).
        const HTTP_EXTENDED_AUTH_PAA = 0x02;
        /// SSPI NTLM extended authentication.
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
    ChannelCloseResponse = 0x11,
    Data = 0x0A,
    ServiceMessage = 0x0B,
    ReauthMessage = 0x0C,
    Keepalive = 0x0D,
}

impl PktTy {
    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn as_u16(self) -> u16 {
        self as u16
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
            0x0A => PktTy::Data,
            0x0B => PktTy::ServiceMessage,
            0x0C => PktTy::ReauthMessage,
            0x0D => PktTy::Keepalive,
            0x10 => PktTy::ChannelClose,
            0x11 => PktTy::ChannelCloseResponse,
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
    pub(crate) const FIXED_PART_SIZE: usize = 2 /* ty */ + 2 /* _reserved */ + 4 /* length */;
}

impl Encode for PktHdr {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.ty.as_u16());
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
    pub extended_auth: HttpExtendedAuth,
}

impl HandshakeRespPkt {
    const FIXED_PART_SIZE: usize = 4 /* error_code */
        + 1 /* ver_major */
        + 1 /* ver_minor */
        + 2 /* server_version */
        + 2 /* extended_auth */;
}

impl Encode for HandshakeRespPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::HandshakeResp,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u32(self.error_code);
        dst.write_u8(self.ver_major);
        dst.write_u8(self.ver_minor);
        dst.write_u16(self.server_version);
        dst.write_u16(self.extended_auth.bits());

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_HANDSHAKE_RESPONSE_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE + Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for HandshakeRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        Ok(HandshakeRespPkt {
            error_code: src.read_u32(),
            ver_major: src.read_u8(),
            ver_minor: src.read_u8(),
            server_version: src.read_u16(),
            // Truncate unknown bits so newer gateways remain interoperable.
            extended_auth: HttpExtendedAuth::from_bits_truncate(src.read_u16()),
        })
    }
}

/// 2.2.5.3.6 `HTTP_TUNNEL_PACKET_FIELD_REAUTH`.
const HTTP_TUNNEL_PACKET_FIELD_REAUTH: u16 = 0x2;

/// 2.2.10.18 HTTP_TUNNEL_PACKET
#[derive(Default)]
pub(crate) struct TunnelReqPkt {
    pub caps: u32,
    pub fields_present: u16,
    pub _reserved: u16,
    /// Tunnel context from an `HTTP_REAUTH_MESSAGE` for a secondary connection.
    pub reauth_tunnel_context: Option<u64>,
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

        let fields_present = self.fields_present
            | if self.reauth_tunnel_context.is_some() {
                HTTP_TUNNEL_PACKET_FIELD_REAUTH
            } else {
                Default::default()
            };
        dst.write_u32(self.caps);
        dst.write_u16(fields_present);
        dst.write_u16(self._reserved);
        if let Some(tunnel_context) = self.reauth_tunnel_context {
            dst.write_u64(tunnel_context);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_TUNNEL_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::default().size() + 8 + self.reauth_tunnel_context.map_or(0, |_| 8)
    }
}

/// 2.2.5.3.9 HTTP_CAPABILITY_TYPE Enumeration
#[repr(u32)]
#[expect(dead_code)]
#[derive(Copy, Clone)]
pub(crate) enum HttpCapsTy {
    QuarSOH = 1,
    IdleTimeout = 2,
    MessagingConsentSign = 4,
    MessagingServiceMsg = 8,
    Reauth = 0x10,
    UdpTransport = 0x20,
}

impl HttpCapsTy {
    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    pub(crate) fn as_u32(self) -> u32 {
        self as u32
    }
}

/// 2.2.5.3.8 HTTP_TUNNEL_RESPONSE_FIELDS_PRESENT_FLAGS
#[repr(u16)]
#[derive(Copy, Clone)]
enum HttpTunnelResponseFields {
    TunnelID = 1,
    Caps = 2,
    /// nonce & server_cert
    Soh = 4,
    Consent = 0x10,
}

impl HttpTunnelResponseFields {
    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn as_u16(self) -> u16 {
        self as u16
    }
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

impl Encode for TunnelRespPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::TunnelResp,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u16(self._server_version);
        dst.write_u32(self.status_code);
        let mut fields_present = self.fields_present;
        fields_present |= u16::from(self.tunnel_id.is_some());
        fields_present |= u16::from(self.caps_flags.is_some()) << 1;
        fields_present |= u16::from(self.nonce.is_some()) << 2;
        fields_present |= u16::from(!self.consent_msg.is_empty()) << 4;
        dst.write_u16(fields_present);
        dst.write_u16(self._reserved);

        if let Some(tunnel_id) = self.tunnel_id {
            dst.write_u32(tunnel_id);
        }
        if let Some(caps_flags) = self.caps_flags {
            dst.write_u32(caps_flags);
        }
        if let Some(nonce) = self.nonce {
            dst.write_u16(nonce);
            let cert_len: u16 = cast_int!("server cert length", self.server_cert.len())?;
            dst.write_u16(cert_len);
            dst.write_slice(&self.server_cert);
        }
        if !self.consent_msg.is_empty() {
            let consent_len: u16 = cast_int!("consent message length", self.consent_msg.len())?;
            dst.write_u16(consent_len);
            dst.write_slice(&self.consent_msg);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_TUNNEL_RESPONSE"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE
            + Self::FIXED_PART_SIZE
            + self.tunnel_id.map_or(0, |_| 4)
            + self.caps_flags.map_or(0, |_| 4)
            + self.nonce.map_or(0, |_| 4 + self.server_cert.len())
            + if self.consent_msg.is_empty() {
                0
            } else {
                2 + self.consent_msg.len()
            }
    }
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

        if pkt.fields_present & (HttpTunnelResponseFields::TunnelID.as_u16()) != 0 {
            ensure_size!(in: src, size: 4);
            pkt.tunnel_id = Some(src.read_u32());
        }
        if pkt.fields_present & (HttpTunnelResponseFields::Caps.as_u16()) != 0 {
            ensure_size!(in: src, size: 4);
            pkt.caps_flags = Some(src.read_u32());
        }
        if pkt.fields_present & (HttpTunnelResponseFields::Soh.as_u16()) != 0 {
            ensure_size!(in: src, size: 2 + 2);
            pkt.nonce = Some(src.read_u16());
            let len = usize::from(src.read_u16());
            ensure_size!(in: src, size: len);
            pkt.server_cert = src.read_slice(len).to_vec();
        }
        if pkt.fields_present & (HttpTunnelResponseFields::Consent.as_u16()) != 0 {
            ensure_size!(in: src, size: 2);
            let len = usize::from(src.read_u16());
            ensure_size!(in: src, size: len);
            pkt.consent_msg = src.read_slice(len).to_vec();
        }

        Ok(pkt)
    }
}

/// 2.2.10.7 HTTP_EXTENDED_AUTH_PACKET Structure
pub(crate) struct ExtendedAuthPkt {
    pub error_code: u32,
    pub blob: Vec<u8>,
}

impl ExtendedAuthPkt {
    pub(crate) fn client_blob(blob: Vec<u8>) -> Self {
        Self { error_code: 0, blob }
    }

    pub(crate) fn error_code(&self) -> u32 {
        self.error_code
    }

    pub(crate) fn blob(&self) -> &[u8] {
        &self.blob
    }
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
        let len = usize::from(src.read_u16());
        ensure_size!(in: src, size: len);

        Ok(ExtendedAuthPkt {
            error_code,
            blob: src.read_slice(len).to_vec(),
        })
    }
}

/// 2.2.5.3.4 HTTP_TUNNEL_AUTH_FIELDS_PRESENT_FLAGS
pub(crate) const HTTP_TUNNEL_AUTH_FIELD_SOH: u16 = 0x1;

/// 2.2.5.3.5 HTTP_TUNNEL_AUTH_RESPONSE_FIELDS_PRESENT_FLAGS
pub(crate) const HTTP_TUNNEL_AUTH_RESPONSE_FIELD_REDIR_FLAGS: u16 = 0x1;
pub(crate) const HTTP_TUNNEL_AUTH_RESPONSE_FIELD_IDLE_TIMEOUT: u16 = 0x2;
pub(crate) const HTTP_TUNNEL_AUTH_RESPONSE_FIELD_SOH_RESPONSE: u16 = 0x4;

/// 2.2.10.14 HTTP_TUNNEL_AUTH_PACKET Structure
pub(crate) struct TunnelAuthPkt {
    pub fields_present: u16,
    pub client_name: String,
    /// Optional statement-of-health blob ([MS-TSGU] 2.2.10.15).
    pub statement_of_health: Option<Vec<u8>>,
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

        let mut fields_present = self.fields_present;
        if self.statement_of_health.is_some() {
            fields_present |= HTTP_TUNNEL_AUTH_FIELD_SOH;
        }
        dst.write_u16(fields_present);

        let client_name_len = self.client_name.encode_utf16().count() * 2 + 2; // Add 2 to account for a null terminator (0x0000).
        let client_name_len: u16 = cast_int!("client name length", client_name_len)?;
        dst.write_u16(client_name_len);

        for c in self.client_name.encode_utf16() {
            dst.write_u16(c);
        }

        dst.write_u16(0);

        if let Some(soh) = &self.statement_of_health {
            // HTTP_byte_BLOB: cbLen (u16) + blob
            let soh_len: u16 = cast_int!("statement of health length", soh.len())?;
            dst.write_u16(soh_len);
            dst.write_slice(soh);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_TUNNEL_AUTH_PACKET"
    }

    fn size(&self) -> usize {
        let soh_size = self
            .statement_of_health
            .as_ref()
            .map(|b| 2 /* cbLen */ + b.len())
            .unwrap_or(0);
        PktHdr::default().size() + 4 + 2 * (self.client_name.len() + 1) + soh_size
    }
}

/// 2.2.10.16 HTTP_TUNNEL_AUTH_RESPONSE Structure (+ optional 2.2.10.17)
#[derive(Debug, Default)]
pub(crate) struct TunnelAuthRespPkt {
    pub(crate) error_code: u32,
    pub(crate) fields_present: u16,
    pub(crate) _reserved: u16,
    /// Device redirection policy flags from the gateway ([MS-TSGU] 2.2.5.3.7).
    pub(crate) redir_flags: Option<u32>,
    /// Idle timeout in minutes when advertised by the gateway.
    pub(crate) idle_timeout_minutes: Option<u32>,
    /// Optional SoH response blob (skipped / retained for diagnostics).
    pub(crate) soh_response: Option<Vec<u8>>,
}

impl TunnelAuthRespPkt {
    const FIXED_PART_SIZE: usize = 4 /* error_code */ + 2 /* fields_present */ + 2 /* _reserved */;

    pub(crate) fn error_code(&self) -> u32 {
        self.error_code
    }
}

impl Encode for TunnelAuthRespPkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::TunnelAuthResponse,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u32(self.error_code);
        let mut fields_present = self.fields_present;
        fields_present |= u16::from(self.redir_flags.is_some());
        fields_present |= u16::from(self.idle_timeout_minutes.is_some()) << 1;
        fields_present |= u16::from(self.soh_response.is_some()) << 2;
        dst.write_u16(fields_present);
        dst.write_u16(self._reserved);

        if let Some(redir_flags) = self.redir_flags {
            dst.write_u32(redir_flags);
        }
        if let Some(idle_timeout_minutes) = self.idle_timeout_minutes {
            dst.write_u32(idle_timeout_minutes);
        }
        if let Some(soh_response) = &self.soh_response {
            let soh_len: u16 = cast_int!("SoH response length", soh_response.len())?;
            dst.write_u16(soh_len);
            dst.write_slice(soh_response);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_TUNNEL_AUTH_RESPONSE"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE
            + Self::FIXED_PART_SIZE
            + self.redir_flags.map_or(0, |_| 4)
            + self.idle_timeout_minutes.map_or(0, |_| 4)
            + self.soh_response.as_ref().map_or(0, |soh| 2 + soh.len())
    }
}

impl Decode<'_> for TunnelAuthRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let mut resp = TunnelAuthRespPkt {
            error_code: src.read_u32(),
            fields_present: src.read_u16(),
            _reserved: src.read_u16(),
            ..TunnelAuthRespPkt::default()
        };

        if resp.fields_present & HTTP_TUNNEL_AUTH_RESPONSE_FIELD_REDIR_FLAGS != 0 {
            ensure_size!(in: src, size: 4);
            resp.redir_flags = Some(src.read_u32());
        }
        if resp.fields_present & HTTP_TUNNEL_AUTH_RESPONSE_FIELD_IDLE_TIMEOUT != 0 {
            ensure_size!(in: src, size: 4);
            resp.idle_timeout_minutes = Some(src.read_u32());
        }
        if resp.fields_present & HTTP_TUNNEL_AUTH_RESPONSE_FIELD_SOH_RESPONSE != 0 {
            ensure_size!(in: src, size: 2);
            let len = usize::from(src.read_u16());
            ensure_size!(in: src, size: len);
            resp.soh_response = Some(src.read_slice(len).to_vec());
        }

        Ok(resp)
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
    pub(crate) error_code: u32,
    pub(crate) fields_present: u16,
    pub(crate) _reserved: u16,

    /// 2.2.10.5 HTTP_CHANNEL_RESPONSE_OPTIONAL
    pub(crate) chan_id: Option<u32>,
    pub(crate) udp_port: u16,
    pub(crate) authn_cookie: Vec<u8>,
}

impl ChannelResp {
    const FIXED_PART_SIZE: usize = 4 /* error_code */ + 2 /* fields_present */ + 2 /* _reserved */;

    pub(crate) fn error_code(&self) -> u32 {
        self.error_code
    }

    /// Successful channel response with a channel id, for test harnesses.
    #[cfg(test)]
    pub(crate) fn success(chan_id: u32) -> Self {
        Self {
            error_code: 0,
            chan_id: Some(chan_id),
            ..ChannelResp::default()
        }
    }

    /// UDP port for a future RDG-UDP side channel, when the gateway supplies one.
    pub(crate) fn udp_port(&self) -> u16 {
        self.udp_port
    }

    /// Authentication cookie for RDG-UDP, when present.
    pub(crate) fn authn_cookie(&self) -> &[u8] {
        &self.authn_cookie
    }
}

impl Encode for ChannelResp {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::ChannelResp,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u32(self.error_code);
        let mut fields_present = self.fields_present;
        fields_present |= u16::from(self.chan_id.is_some());
        fields_present |= u16::from(self.udp_port != 0) << 1;
        fields_present |= u16::from(!self.authn_cookie.is_empty()) << 2;
        dst.write_u16(fields_present);
        dst.write_u16(self._reserved);

        if let Some(chan_id) = self.chan_id {
            dst.write_u32(chan_id);
        }
        if self.udp_port != 0 {
            dst.write_u16(self.udp_port);
        }
        if !self.authn_cookie.is_empty() {
            let cookie_len: u16 = cast_int!("authn cookie length", self.authn_cookie.len())?;
            dst.write_u16(cookie_len);
            dst.write_slice(&self.authn_cookie);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_CHANNEL_RESPONSE"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE
            + Self::FIXED_PART_SIZE
            + self.chan_id.map_or(0, |_| 4)
            + if self.udp_port == 0 { 0 } else { 2 }
            + if self.authn_cookie.is_empty() {
                0
            } else {
                2 + self.authn_cookie.len()
            }
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
            let len = usize::from(src.read_u16());
            ensure_size!(in: src, size: len);
            resp.authn_cookie = src.read_slice(len).to_vec();
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
        let len = usize::from(src.read_u16());
        ensure_size!(in: src, size: len);
        Ok(DataPkt {
            data: src.read_slice(len),
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

/// 2.2.10.13 HTTP_SERVICE_MESSAGE Structure (body only; header stripped).
#[derive(Debug)]
pub(crate) struct ServiceMessagePkt {
    pub message: String,
}

impl Encode for ServiceMessagePkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::ServiceMessage,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        let utf16_len = self.message.encode_utf16().count() * 2 + 2 /* NUL */;
        let utf16_len = cast_int!("service message length", utf16_len)?;
        dst.write_u16(utf16_len);
        for c in self.message.encode_utf16() {
            dst.write_u16(c);
        }
        dst.write_u16(0);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_SERVICE_MESSAGE"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE + 2 + 2 * (self.message.encode_utf16().count() + 1)
    }
}

impl Decode<'_> for ServiceMessagePkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 2);
        let cb_message = usize::from(src.read_u16());
        ensure_size!(in: src, size: cb_message);
        let raw = src.read_slice(cb_message);
        let message = String::from_utf16_lossy(
            &raw.chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        )
        .trim_end_matches('\0')
        .to_owned();
        Ok(Self { message })
    }
}

/// 2.2.10.12 HTTP_REAUTH_MESSAGE Structure (body only; header stripped).
#[derive(Debug)]
pub(crate) struct ReauthMessagePkt {
    pub reauth_tunnel_context: u64,
}

impl Encode for ReauthMessagePkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::ReauthMessage,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u64(self.reauth_tunnel_context);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_REAUTH_MESSAGE"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE + 8
    }
}

impl Decode<'_> for ReauthMessagePkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 8);
        Ok(Self {
            reauth_tunnel_context: src.read_u64(),
        })
    }
}

/// 2.2.10.15 HTTP_CLOSE_PACKET Structure (body only; header stripped).
#[derive(Debug)]
pub(crate) struct ChannelClosePkt {
    pub status_code: u32,
}

impl Encode for ChannelClosePkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty: PktTy::ChannelClose,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;

        dst.write_u32(self.status_code);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_CLOSE_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE + 4
    }
}

impl Decode<'_> for ChannelClosePkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 4);
        Ok(Self {
            status_code: src.read_u32(),
        })
    }
}

/// Human-readable label for common MS-TSGU HRESULTs ([MS-TSGU] 2.2.6).
pub(crate) fn gateway_code_label(code: u32) -> Option<&'static str> {
    // Normalize success-style low-word codes that may arrive without the FACILITY bit.
    let c = code;
    Some(match c {
        0x0000_0000 => "ERROR_SUCCESS",
        0x0000_0005 => "ERROR_ACCESS_DENIED",
        0x0000_00A0 => "ERROR_BAD_ARGUMENTS",
        0x0000_04CA => "ERROR_GRACEFUL_DISCONNECT",
        0x0000_04D4 => "E_PROXY_CONNECTIONABORTED",
        0x0000_59D8 | 0x8007_59D8 => "E_PROXY_INTERNALERROR",
        0x0000_59DA | 0x8007_59DA => "E_PROXY_RAP_ACCESSDENIED",
        0x0000_59DB | 0x8007_59DB => "E_PROXY_NAP_ACCESSDENIED",
        0x0000_59DD => "E_PROXY_TS_CONNECTFAILED",
        0x0000_59DF | 0x8007_59DF => "E_PROXY_ALREADYDISCONNECTED",
        0x0000_59E6 => "E_PROXY_MAXCONNECTIONSREACHED",
        0x0000_59E8 => "E_PROXY_NOTSUPPORTED",
        0x0000_59E9 | 0x8007_59E9 => "E_PROXY_CAPABILITYMISMATCH",
        0x0000_59ED | 0x8007_59ED => "E_PROXY_QUARANTINE_ACCESSDENIED",
        0x0000_59EE | 0x8007_59EE => "E_PROXY_NOCERTAVAILABLE",
        0x0000_59F6 => "E_PROXY_SESSIONTIMEOUT",
        0x0000_59F7 | 0x8007_59F7 => "E_PROXY_COOKIE_BADPACKET",
        0x0000_59F8 | 0x8007_59F8 => "E_PROXY_COOKIE_AUTHENTICATION_ACCESS_DENIED",
        0x0000_59F9 | 0x8007_59F9 => "E_PROXY_UNSUPPORTED_AUTHENTICATION_METHOD",
        0x0000_59FA => "E_PROXY_REAUTH_AUTHN_FAILED",
        0x0000_59FB => "E_PROXY_REAUTH_CAP_FAILED",
        0x0000_59FC => "E_PROXY_REAUTH_RAP_FAILED",
        0x0000_59FD => "E_PROXY_SDR_NOT_SUPPORTED_BY_TS",
        0x0000_5A00 => "E_PROXY_REAUTH_NAP_FAILED",
        0x8009_030C => "SEC_E_LOGON_DENIED",
        _ => return None,
    })
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
    fn http_extended_auth_none_is_zero() {
        assert_eq!(HttpExtendedAuth::HTTP_EXTENDED_AUTH_NONE.bits(), 0x00);
        assert_eq!(HttpExtendedAuth::HTTP_EXTENDED_AUTH_SC.bits(), 0x01);
        assert_eq!(HttpExtendedAuth::HTTP_EXTENDED_AUTH_PAA.bits(), 0x02);
        assert_eq!(HttpExtendedAuth::HTTP_EXTENDED_AUTH_SSPI_NTLM.bits(), 0x04);
        assert_eq!(HttpExtendedAuth::empty().bits(), 0x00);
    }

    #[test]
    fn pkt_hdr_size_matches_wire_layout() {
        assert_eq!(PktHdr::FIXED_PART_SIZE, 8);
        let hdr = PktHdr {
            ty: PktTy::Keepalive,
            _reserved: 0,
            length: 8,
        };
        let bytes = encode_to_vec(&hdr);
        assert_eq!(bytes, [0x0d, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00]);

        let mut cur = ReadCursor::new(&bytes);
        let decoded = PktHdr::decode(&mut cur).expect("decode header");
        assert_eq!(decoded.ty, PktTy::Keepalive);
        assert_eq!(decoded.length, 8);
        assert!(cur.eof());
    }

    #[test]
    fn handshake_req_encode_layout() {
        let pkt = HandshakeReqPkt {
            ver_major: 1,
            ver_minor: 0,
            client_version: 0,
            extended_auth: HttpExtendedAuth::HTTP_EXTENDED_AUTH_NONE,
        };
        let bytes = encode_to_vec(&pkt);
        assert_eq!(bytes.len(), 14);
        assert_eq!(&bytes[..8], &[0x01, 0x00, 0x00, 0x00, 0x0e, 0x00, 0x00, 0x00]);
        assert_eq!(&bytes[8..], &[0x01, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn handshake_resp_decode_layout() {
        // HTTP_PACKET_HEADER is stripped before HandshakeRespPkt::decode in the client.
        let body = [
            0x00, 0x00, 0x00, 0x00, // error_code
            0x01, // ver_major
            0x00, // ver_minor
            0x00, 0x00, // server_version
            0x00, 0x00, // extended_auth = NONE
        ];
        assert_eq!(body.len(), HandshakeRespPkt::FIXED_PART_SIZE);

        let mut cur = ReadCursor::new(&body);
        let resp = HandshakeRespPkt::decode(&mut cur).expect("decode handshake response");
        assert_eq!(resp.error_code, 0);
        assert_eq!(resp.ver_major, 1);
        assert_eq!(resp.ver_minor, 0);
        assert_eq!(resp.server_version, 0);
        assert_eq!(resp.extended_auth, HttpExtendedAuth::HTTP_EXTENDED_AUTH_NONE);
        assert!(cur.eof());
    }

    #[test]
    fn extended_auth_pkt_roundtrip() {
        let pkt = ExtendedAuthPkt::client_blob(vec![0x4e, 0x54, 0x4c, 0x4d]);
        let bytes = encode_to_vec(&pkt);
        let mut cur = ReadCursor::new(&bytes);
        let hdr = PktHdr::decode(&mut cur).expect("header");
        assert_eq!(hdr.ty, PktTy::ExtendedAuth);
        let decoded = ExtendedAuthPkt::decode(&mut cur).expect("body");
        assert_eq!(decoded.error_code(), 0);
        assert_eq!(decoded.blob(), &[0x4e, 0x54, 0x4c, 0x4d]);
        assert!(cur.eof());
    }

    #[test]
    fn reauth_and_service_message_decode() {
        let reauth_body = 0x1122_3344_5566_7788u64.to_le_bytes();
        let mut cur = ReadCursor::new(&reauth_body);
        let reauth = ReauthMessagePkt::decode(&mut cur).expect("reauth");
        assert_eq!(reauth.reauth_tunnel_context, 0x1122_3344_5566_7788);

        // "OK\0" as UTF-16LE
        let service_body = [0x06, 0x00, b'O', 0x00, b'K', 0x00, 0x00, 0x00];
        let mut cur = ReadCursor::new(&service_body);
        let service = ServiceMessagePkt::decode(&mut cur).expect("service");
        assert_eq!(service.message, "OK");
    }

    #[test]
    fn tunnel_request_encodes_reauth_context() {
        let bytes = encode_to_vec(&TunnelReqPkt {
            caps: HttpCapsTy::Reauth.as_u32(),
            fields_present: 0,
            _reserved: 0,
            reauth_tunnel_context: Some(0x1122_3344_5566_7788),
        });
        assert_eq!(bytes.len(), PktHdr::default().size() + 16);
        assert_eq!(
            u16::from_le_bytes(bytes[0..2].try_into().unwrap()),
            PktTy::TunnelCreate.as_u16()
        );
        assert_eq!(
            u16::from_le_bytes(bytes[12..14].try_into().unwrap()),
            HTTP_TUNNEL_PACKET_FIELD_REAUTH
        );
        assert_eq!(
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            0x1122_3344_5566_7788
        );
    }

    #[test]
    fn gateway_code_label_known() {
        assert_eq!(gateway_code_label(0), Some("ERROR_SUCCESS"));
        assert_eq!(gateway_code_label(0x8007_59DA), Some("E_PROXY_RAP_ACCESSDENIED"));
        assert_eq!(gateway_code_label(0xDEAD_BEEF), None);
    }

    #[test]
    fn channel_pkt_encodes_port_and_resources() {
        let pkt = ChannelPkt {
            resources: vec!["rdp.example".to_owned()],
            port: 2179,
            protocol: 3,
        };
        let bytes = encode_to_vec(&pkt);

        // Header type ChannelCreate = 0x08, length = full packet size.
        assert_eq!(&bytes[..2], &[0x08, 0x00]);
        let length = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(usize::try_from(length).unwrap(), bytes.len());

        let body = &bytes[8..];
        assert_eq!(body[0], 1); // resources_count
        assert_eq!(body[1], 0); // alt_names
        assert_eq!(u16::from_le_bytes(body[2..4].try_into().unwrap()), 2179);
        assert_eq!(u16::from_le_bytes(body[4..6].try_into().unwrap()), 3);

        // resource name length is UTF-16 bytes including NUL.
        let name_len = usize::from(u16::from_le_bytes(body[6..8].try_into().unwrap()));
        assert_eq!(name_len, ("rdp.example".len() + 1) * 2);
        assert_eq!(body.len(), 6 + 2 + name_len);
    }

    #[test]
    fn channel_resp_decode_error_code() {
        let body = [
            0x05, 0x00, 0x00, 0x80, // error_code (HRESULT-style)
            0x00, 0x00, // fields_present
            0x00, 0x00, // reserved
        ];
        let mut cur = ReadCursor::new(&body);
        let resp = ChannelResp::decode(&mut cur).expect("decode channel response");
        assert_eq!(resp.error_code(), 0x8000_0005);
        assert!(cur.eof());
    }

    #[test]
    fn data_pkt_roundtrip() {
        let payload = b"hello-rdg";
        let pkt = DataPkt { data: payload };
        let bytes = encode_to_vec(&pkt);

        let mut cur = ReadCursor::new(&bytes);
        let hdr = PktHdr::decode(&mut cur).expect("header");
        assert_eq!(hdr.ty, PktTy::Data);
        assert_eq!(usize::try_from(hdr.length).unwrap(), bytes.len());

        let decoded = DataPkt::decode(&mut cur).expect("data");
        assert_eq!(decoded.data, payload);
        assert!(cur.eof());
    }

    #[test]
    fn keepalive_encode() {
        let bytes = encode_to_vec(&KeepalivePkt);
        assert_eq!(bytes, [0x0d, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn tunnel_auth_pkt_encodes_optional_soh() {
        let pkt = TunnelAuthPkt {
            fields_present: 0,
            client_name: "PC1".to_owned(),
            statement_of_health: Some(vec![0xaa, 0xbb]),
        };
        let bytes = encode_to_vec(&pkt);
        let mut cur = ReadCursor::new(&bytes);
        let hdr = PktHdr::decode(&mut cur).expect("header");
        assert_eq!(hdr.ty, PktTy::TunnelAuth);
        assert_eq!(usize::try_from(hdr.length).unwrap(), bytes.len());

        let fields = cur.read_u16();
        assert_eq!(fields & HTTP_TUNNEL_AUTH_FIELD_SOH, HTTP_TUNNEL_AUTH_FIELD_SOH);
        let name_len = usize::from(cur.read_u16());
        assert_eq!(name_len, ("PC1".len() + 1) * 2);
        cur.read_slice(name_len);
        let soh_len = usize::from(cur.read_u16());
        assert_eq!(soh_len, 2);
        assert_eq!(cur.read_slice(2), &[0xaa, 0xbb]);
        assert!(cur.eof());
    }

    #[test]
    fn tunnel_auth_resp_decodes_optional_fields() {
        let mut body = vec![
            0x00, 0x00, 0x00, 0x00, // error_code
            0x07, 0x00, // fields: redir | idle | soh
            0x00, 0x00, // reserved
            0x00, 0x00, 0x00, 0x80, // redir_flags = HTTP_TUNNEL_REDIR_ENABLE_ALL
            0x0f, 0x00, 0x00, 0x00, // idle_timeout = 15 minutes
            0x03, 0x00, // soh_response cbLen
            0x01, 0x02, 0x03,
        ];
        let mut cur = ReadCursor::new(&body);
        let resp = TunnelAuthRespPkt::decode(&mut cur).expect("decode");
        assert_eq!(resp.error_code(), 0);
        assert_eq!(resp.redir_flags, Some(0x8000_0000));
        assert_eq!(resp.idle_timeout_minutes, Some(15));
        assert_eq!(resp.soh_response.as_deref(), Some(&[1, 2, 3][..]));
        assert!(cur.eof());

        // Fixed-only response still works.
        body.truncate(8);
        body[4] = 0;
        body[5] = 0;
        let mut cur = ReadCursor::new(&body);
        let resp = TunnelAuthRespPkt::decode(&mut cur).expect("fixed");
        assert!(resp.redir_flags.is_none());
        assert!(resp.idle_timeout_minutes.is_none());
        assert!(resp.soh_response.is_none());
    }

    #[test]
    fn channel_resp_decodes_udp_cookie() {
        let body = [
            0x00, 0x00, 0x00, 0x00, // error
            0x06, 0x00, // fields: udp | cookie
            0x00, 0x00, // reserved
            0x39, 0x30, // udp_port 12345
            0x02, 0x00, // cookie len
            0xde, 0xad,
        ];
        let mut cur = ReadCursor::new(&body);
        let resp = ChannelResp::decode(&mut cur).expect("decode");
        assert_eq!(resp.udp_port(), 12345);
        assert_eq!(resp.authn_cookie(), &[0xde, 0xad]);
        assert!(cur.eof());
    }

    /// Encode the client request sequence and decode a synthetic multi-stage gateway response
    /// stream (handshake → tunnel → tunnel auth → channel), matching the WebSocket data path.
    #[test]
    fn client_request_and_server_response_sequence() {
        let handshake_req = encode_to_vec(&HandshakeReqPkt {
            ver_major: 1,
            ver_minor: 0,
            client_version: 0,
            extended_auth: HttpExtendedAuth::HTTP_EXTENDED_AUTH_SSPI_NTLM,
        });
        let tunnel_req = encode_to_vec(&TunnelReqPkt {
            caps: HttpCapsTy::MessagingConsentSign.as_u32()
                | HttpCapsTy::MessagingServiceMsg.as_u32()
                | HttpCapsTy::IdleTimeout.as_u32()
                | HttpCapsTy::Reauth.as_u32(),
            fields_present: 0,
            _reserved: 0,
            reauth_tunnel_context: None,
        });
        let tunnel_auth_req = encode_to_vec(&TunnelAuthPkt {
            fields_present: 0,
            client_name: "IRONRDP".to_owned(),
            statement_of_health: None,
        });
        let channel_req = encode_to_vec(&ChannelPkt {
            resources: vec!["target.example".to_owned()],
            port: 3389,
            protocol: 3,
        });

        assert_eq!(
            u16::from_le_bytes(handshake_req[0..2].try_into().unwrap()),
            PktTy::HandshakeReq.as_u16()
        );
        assert_eq!(
            u16::from_le_bytes(tunnel_req[0..2].try_into().unwrap()),
            PktTy::TunnelCreate.as_u16()
        );
        assert_eq!(
            u16::from_le_bytes(tunnel_auth_req[0..2].try_into().unwrap()),
            PktTy::TunnelAuth.as_u16()
        );
        assert_eq!(
            u16::from_le_bytes(channel_req[0..2].try_into().unwrap()),
            PktTy::ChannelCreate.as_u16()
        );

        // Server responses as concatenated framed packets (header + body).
        let mut stream = Vec::new();

        // Handshake response (header + body).
        stream.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00]); // HandshakeResp, len 18
        stream.extend_from_slice(&[
            0x00, 0x00, 0x00, 0x00, // error
            0x01, 0x00, // ver
            0x00, 0x00, // server_version
            0x00, 0x00, // extended_auth NONE
        ]);

        // Tunnel response with tunnel id + caps (ConsentSign | IdleTimeout).
        let tunnel_body = [
            0x01, 0x00, // server_version
            0x00, 0x00, 0x00, 0x00, // status
            0x03, 0x00, // fields: tunnel_id | caps
            0x00, 0x00, // reserved
            0x11, 0x22, 0x33, 0x44, // tunnel_id
            0x06, 0x00, 0x00, 0x00, // caps = ConsentSign | IdleTimeout
        ];
        let tunnel_len = u32::try_from(8 + tunnel_body.len()).unwrap();
        stream.extend_from_slice(&[0x05, 0x00, 0x00, 0x00]); // TunnelResp
        stream.extend_from_slice(&tunnel_len.to_le_bytes());
        stream.extend_from_slice(&tunnel_body);

        // Tunnel auth response fixed-only.
        stream.extend_from_slice(&[0x07, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00]); // TunnelAuthResponse, len 16
        stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        // Channel response success.
        stream.extend_from_slice(&[0x09, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00]); // ChannelResp, len 16
        stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        let mut cur = ReadCursor::new(&stream);

        let hdr = PktHdr::decode(&mut cur).expect("hs hdr");
        assert_eq!(hdr.ty, PktTy::HandshakeResp);
        let hs = HandshakeRespPkt::decode(&mut cur).expect("hs body");
        assert_eq!(hs.error_code, 0);
        assert_eq!(hs.extended_auth, HttpExtendedAuth::HTTP_EXTENDED_AUTH_NONE);

        let hdr = PktHdr::decode(&mut cur).expect("tunnel hdr");
        assert_eq!(hdr.ty, PktTy::TunnelResp);
        let tunnel = TunnelRespPkt::decode(&mut cur).expect("tunnel body");
        assert_eq!(tunnel.status_code, 0);
        assert_eq!(tunnel.tunnel_id, Some(0x4433_2211));
        assert_eq!(
            tunnel.caps_flags,
            Some(HttpCapsTy::MessagingConsentSign.as_u32() | HttpCapsTy::IdleTimeout.as_u32())
        );

        let hdr = PktHdr::decode(&mut cur).expect("auth hdr");
        assert_eq!(hdr.ty, PktTy::TunnelAuthResponse);
        let auth = TunnelAuthRespPkt::decode(&mut cur).expect("auth body");
        assert_eq!(auth.error_code(), 0);

        let hdr = PktHdr::decode(&mut cur).expect("channel hdr");
        assert_eq!(hdr.ty, PktTy::ChannelResp);
        let channel = ChannelResp::decode(&mut cur).expect("channel body");
        assert_eq!(channel.error_code(), 0);
        assert!(cur.eof());
    }
}
