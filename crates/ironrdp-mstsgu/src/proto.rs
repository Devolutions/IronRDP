use bitflags::bitflags;
use ironrdp_core::{
    Decode, Encode, ReadCursor, WriteCursor, cast_int, cast_length, ensure_fixed_part_size, ensure_size,
    unsupported_value_err,
};

bitflags! {
    /// 2.2.5.3.2 HTTP_EXTENDED_AUTH Enumeration
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) struct HttpExtendedAuth: u16 {
        const HTTP_EXTENDED_AUTH_NONE = 0x00;
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
    const FIXED_PART_SIZE: usize = 4 /* ty */ + 2 /* _reserved */ + 2 /* length */;
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
        let mty =
            PktTy::try_from(ty).map_err(|_| unsupported_value_err("PktHdr::ty", "ty", format!("0x{ty:x}"), None))?;

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
    #[expect(
        dead_code,
        reason = "authentication flow does not negotiate extended authentication yet"
    )]
    pub extended_auth: HttpExtendedAuth,
}

impl HandshakeRespPkt {
    const FIXED_PART_SIZE: usize =
        4 /* error_code */ + 1 /* ver_major */ + 1 /* ver_minor */ + 2 /* server_version */ + 2 /* extended_auth */;
}

impl Decode<'_> for HandshakeRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        Ok(HandshakeRespPkt {
            error_code: src.read_u32(),
            ver_major: src.read_u8(),
            ver_minor: src.read_u8(),
            server_version: src.read_u16(),
            extended_auth: HttpExtendedAuth::from_bits_retain(src.read_u16()),
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

        let fields_present = self.fields_present & !HTTP_TUNNEL_PACKET_FIELD_REAUTH
            | if self.reauth_tunnel_context.is_some() {
                HTTP_TUNNEL_PACKET_FIELD_REAUTH
            } else {
                0
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
        PktHdr::FIXED_PART_SIZE
            + 4 /* capsFlags */
            + 2 /* fieldsPresent */
            + 2 /* reserved */
            + self.reauth_tunnel_context.map_or(0, |_| 8 /* reauthTunnelContext */)
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "authentication flow does not process extended authentication yet"
    )
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtendedAuthPkt {
    pub(crate) error_code: u32,
    pub(crate) auth_blob: Vec<u8>,
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
        let auth_blob_len: u16 = cast_int!("auth blob length", self.auth_blob.len())?;
        dst.write_u16(auth_blob_len);
        dst.write_slice(&self.auth_blob);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "HTTP_EXTENDED_AUTH_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE + 4 /* error_code */ + 2 /* cb_blob_len */ + self.auth_blob.len()
    }
}

impl Decode<'_> for ExtendedAuthPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* error_code */ + 2 /* cb_blob_len */);
        let error_code = src.read_u32();
        let auth_blob_len = usize::from(src.read_u16());
        ensure_size!(in: src, size: auth_blob_len);

        Ok(ExtendedAuthPkt {
            error_code,
            auth_blob: src.read_slice(auth_blob_len).to_vec(),
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

/// [2.2.5.3.5] `HTTP_TUNNEL_AUTH_RESPONSE_FIELDS_PRESENT_FLAGS`.
///
/// [2.2.5.3.5]: https://winprotocoldocs-bhdugrdyduf5h2e4.b02.azurefd.net/MS-TSGU/%5bMS-TSGU%5d.pdf#page=42
const HTTP_TUNNEL_AUTH_RESPONSE_FIELD_REDIR_FLAGS: u16 = 0x1;
const HTTP_TUNNEL_AUTH_RESPONSE_FIELD_IDLE_TIMEOUT: u16 = 0x2;
const HTTP_TUNNEL_AUTH_RESPONSE_FIELD_SOH_RESPONSE: u16 = 0x4;

/// [2.2.10.16] `HTTP_TUNNEL_AUTH_RESPONSE` structure and [2.2.10.17] optional fields.
///
/// [2.2.10.16]: https://winprotocoldocs-bhdugrdyduf5h2e4.b02.azurefd.net/MS-TSGU/%5bMS-TSGU%5d.pdf#page=70
/// [2.2.10.17]: https://winprotocoldocs-bhdugrdyduf5h2e4.b02.azurefd.net/MS-TSGU/%5bMS-TSGU%5d.pdf#page=70
#[derive(Debug, Default)]
pub(crate) struct TunnelAuthRespPkt {
    pub(crate) error_code: u32,
    fields_present: u16,
    _reserved: u16,
    pub(crate) redirection_flags: Option<u32>,
    pub(crate) idle_timeout_minutes: Option<u32>,
    pub(crate) soh_response: Option<Vec<u8>>,
}

impl TunnelAuthRespPkt {
    const FIXED_PART_SIZE: usize = 4 /* error_code */ + 2 /* fields_present */ + 2 /* _reserved */;
}

impl Decode<'_> for TunnelAuthRespPkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let mut pkt = TunnelAuthRespPkt {
            error_code: src.read_u32(),
            fields_present: src.read_u16(),
            _reserved: src.read_u16(),
            ..TunnelAuthRespPkt::default()
        };

        if pkt.fields_present & HTTP_TUNNEL_AUTH_RESPONSE_FIELD_REDIR_FLAGS != 0 {
            ensure_size!(in: src, size: 4 /* redirFlags */);
            pkt.redirection_flags = Some(src.read_u32());
        }
        if pkt.fields_present & HTTP_TUNNEL_AUTH_RESPONSE_FIELD_IDLE_TIMEOUT != 0 {
            ensure_size!(in: src, size: 4 /* idleTimeout */);
            pkt.idle_timeout_minutes = Some(src.read_u32());
        }
        if pkt.fields_present & HTTP_TUNNEL_AUTH_RESPONSE_FIELD_SOH_RESPONSE != 0 {
            ensure_size!(in: src, size: 2 /* cbLen */);
            let soh_response_len = usize::from(src.read_u16());
            ensure_size!(in: src, size: soh_response_len);
            pkt.soh_response = Some(src.read_slice(soh_response_len).to_vec());
        }

        Ok(pkt)
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

/// [2.2.10.13] `HTTP_SERVICE_MESSAGE` structure (body only; header stripped).
///
/// [2.2.10.13]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceMessagePkt {
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
        let utf16_len = cast_int!("cbMessageLen", utf16_len)?;
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
        PktHdr::FIXED_PART_SIZE + 2 /* cbMessageLen */ + 2 * (self.message.encode_utf16().count() + 1 /* NUL */)
    }
}

impl Decode<'_> for ServiceMessagePkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 2 /* cbMessageLen */);
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

/// [2.2.10.12] `HTTP_REAUTH_MESSAGE` structure (body only; header stripped).
///
/// [2.2.10.12]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReauthMessagePkt {
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
        PktHdr::FIXED_PART_SIZE + 8 /* reauthTunnelContext */
    }
}

impl Decode<'_> for ReauthMessagePkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 8 /* reauthTunnelContext */);
        Ok(Self {
            reauth_tunnel_context: src.read_u64(),
        })
    }
}

/// [2.2.10.23] `HTTP_CLOSE_PACKET` structure (body only; header stripped).
///
/// [2.2.10.23]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelClosePkt {
    pub status_code: u32,
}

impl ChannelClosePkt {
    pub(crate) fn encode_as(&self, ty: PktTy, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let hdr = PktHdr {
            ty,
            length: cast_int!("packet length", self.size())?,
            ..PktHdr::default()
        };
        hdr.encode(dst)?;
        dst.write_u32(self.status_code);
        Ok(())
    }
}

impl Encode for ChannelClosePkt {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        self.encode_as(PktTy::ChannelClose, dst)
    }

    fn name(&self) -> &'static str {
        "HTTP_CLOSE_PACKET"
    }

    fn size(&self) -> usize {
        PktHdr::FIXED_PART_SIZE + 4 /* statusCode */
    }
}

impl Decode<'_> for ChannelClosePkt {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 4 /* statusCode */);
        Ok(Self {
            status_code: src.read_u32(),
        })
    }
}

/// Human-readable label for common MS-TSGU HRESULTs ([2.2.6]).
///
/// [2.2.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
pub fn gateway_code_label(code: u32) -> Option<&'static str> {
    Some(match code {
        0x0000_0000 => "ERROR_SUCCESS",
        0x0000_0005 => "ERROR_ACCESS_DENIED",
        0x0000_00A0 => "ERROR_BAD_ARGUMENTS",
        0x0000_04CA => "ERROR_GRACEFUL_DISCONNECT",
        0x0000_04D4 => "E_PROXY_CONNECTIONABORTED",
        0x0000_59D8 | 0x8007_59D8 => "E_PROXY_INTERNALERROR",
        0x0000_59DA | 0x8007_59DA => "E_PROXY_RAP_ACCESSDENIED",
        0x0000_59DB | 0x8007_59DB => "E_PROXY_NAP_ACCESSDENIED",
        0x0000_59DD | 0x8007_59DD => "E_PROXY_TS_CONNECTFAILED",
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

    #[test]
    fn channel_pkt_encodes_requested_port() {
        let pkt = ChannelPkt {
            resources: vec!["rdp.example.com".to_owned()],
            port: 2179,
            protocol: 3,
        };
        let mut buf = vec![0; pkt.size()];
        let mut dst = WriteCursor::new(&mut buf);
        pkt.encode(&mut dst).expect("encode");

        // HTTP_PACKET_HEADER is 8 bytes; the next two bytes are resources/alt_names counts.
        assert_eq!(&buf[10..12], &2179u16.to_le_bytes());
    }

    #[test]
    fn extended_auth_pkt_size_includes_header_and_blob() {
        let pkt = ExtendedAuthPkt {
            error_code: 0,
            auth_blob: vec![0; 3],
        };

        assert_eq!(
            pkt.size(),
            PktHdr::FIXED_PART_SIZE + 4 /* error_code */ + 2 /* cb_blob_len */ + 3 /* auth_blob */
        );
    }
}
