//! RDP multitransport bootstrapping and tunnel PDU types.
//!
//! The bootstrapping PDUs are defined in [\[MS-RDPBCGR\] 2.2.15.1] and
//! [\[MS-RDPBCGR\] 2.2.15.2]. Tunnel PDUs are defined in [\[MS-RDPEMT\] 2.2.1]
//! and [\[MS-RDPEMT\] 2.2.2].
//!
//! [\[MS-RDPBCGR\] 2.2.15.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/de783158-8b01-4818-8fb0-62523a5b3490
//! [\[MS-RDPBCGR\] 2.2.15.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/44044233-e498-46f8-8e16-1ffa595a8e8b
//! [\[MS-RDPEMT\] 2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/d22b606c-32c4-4647-b356-86f75e23a22c
//! [\[MS-RDPEMT\] 2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/d22b606c-32c4-4647-b356-86f75e23a22c

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err, read_padding, write_padding,
};

use crate::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};

/// Length of the security cookie used for transport binding validation.
const SECURITY_COOKIE_LEN: usize = 16;

/// An RDPEMT tunnel subheader.
///
/// Defined in [\[MS-RDPEMT\] 2.2.1.1.1].
///
/// [\[MS-RDPEMT\] 2.2.1.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/d22b606c-32c4-4647-b356-86f75e23a22c
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct TunnelSubheader {
    /// Type of the encapsulated subheader.
    pub ty: u8,
    /// Type-specific bytes following the length and type fields.
    pub data: Vec<u8>,
}

impl TunnelSubheader {
    const FIXED_PART_SIZE: usize = 1 /* subHeaderLength */ + 1 /* subHeaderType */;

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data.len()
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let length: u8 = cast_length!("subHeaderLength", self.size())?;

        dst.write_u8(length);
        dst.write_u8(self.ty);
        dst.write_slice(&self.data);

        Ok(())
    }

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::FIXED_PART_SIZE);

        let length = usize::from(src.read_u8());
        if length < Self::FIXED_PART_SIZE {
            return Err(invalid_field_err!("subHeaderLength", "must be at least two bytes"));
        }

        ensure_size!(in: src, size: length - 1 /* subHeaderLength */);
        let ty = src.read_u8();
        let data = src.read_slice(length - Self::FIXED_PART_SIZE).to_vec();

        Ok(Self { ty, data })
    }
}

/// Action carried in an RDPEMT tunnel PDU header.
///
/// Defined in [\[MS-RDPEMT\] 2.2.1.1].
///
/// [\[MS-RDPEMT\] 2.2.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/d22b606c-32c4-4647-b356-86f75e23a22c
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[repr(u8)]
pub enum TunnelAction {
    /// `RDP_TUNNEL_CREATEREQUEST`.
    CreateRequest = 0x0,
    /// `RDP_TUNNEL_CREATERESPONSE`.
    CreateResponse = 0x1,
    /// `RDP_TUNNEL_DATA`.
    Data = 0x2,
}

impl TunnelAction {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x0 => Some(Self::CreateRequest),
            0x1 => Some(Self::CreateResponse),
            0x2 => Some(Self::Data),
            _ => None,
        }
    }

    #[expect(
        clippy::as_conversions,
        reason = "repr(u8) guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TunnelHeader {
    action: TunnelAction,
    subheaders: Vec<TunnelSubheader>,
}

impl TunnelHeader {
    const FIXED_PART_SIZE: usize = 1 /* action and flags */ + 2 /* payloadLength */ + 1 /* headerLength */;

    fn encoded_size(subheaders: &[TunnelSubheader]) -> usize {
        Self::FIXED_PART_SIZE + subheaders.iter().map(TunnelSubheader::size).sum::<usize>()
    }

    fn encode(
        action: TunnelAction,
        subheaders: &[TunnelSubheader],
        payload_length: usize,
        dst: &mut WriteCursor<'_>,
    ) -> EncodeResult<()> {
        let payload_length: u16 = cast_length!("payloadLength", payload_length)?;
        let header_length: u8 = cast_length!("headerLength", Self::encoded_size(subheaders))?;
        ensure_size!(in: dst, size: usize::from(header_length) + usize::from(payload_length));

        dst.write_u8(action.as_u8());
        dst.write_u16(payload_length);
        dst.write_u8(header_length);
        subheaders.iter().try_for_each(|subheader| subheader.encode(dst))
    }

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<(Self, usize)> {
        ensure_size!(in: src, size: Self::FIXED_PART_SIZE);

        let action_and_flags = src.read_u8();
        let flags = action_and_flags >> 4;
        if flags != 0 {
            return Err(invalid_field_err!("flags", "must be zero"));
        }

        let action = TunnelAction::from_u8(action_and_flags & 0x0F)
            .ok_or_else(|| invalid_field_err!("action", "unknown tunnel action"))?;
        let payload_length = usize::from(src.read_u16());
        let header_length = usize::from(src.read_u8());
        if header_length < Self::FIXED_PART_SIZE {
            return Err(invalid_field_err!("headerLength", "must be at least four bytes"));
        }

        let subheader_length = header_length - Self::FIXED_PART_SIZE;
        ensure_size!(in: src, size: subheader_length);
        let mut subheaders_cursor = ReadCursor::new(src.read_slice(subheader_length));
        let mut subheaders = Vec::new();
        while !subheaders_cursor.is_empty() {
            subheaders.push(TunnelSubheader::decode(&mut subheaders_cursor)?);
        }

        ensure_size!(in: src, size: payload_length);

        Ok((Self { action, subheaders }, payload_length))
    }
}

/// Client request that binds an RDPEMT tunnel to the main RDP connection.
///
/// Defined in [\[MS-RDPEMT\] 2.2.2.1].
///
/// [\[MS-RDPEMT\] 2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/d22b606c-32c4-4647-b356-86f75e23a22c
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct TunnelCreateRequestPdu {
    /// Request ID from the Initiate Multitransport Request PDU.
    pub request_id: u32,
    /// Security cookie from the Initiate Multitransport Request PDU.
    pub security_cookie: [u8; SECURITY_COOKIE_LEN],
}

impl TunnelCreateRequestPdu {
    const PAYLOAD_SIZE: usize = 4 /* requestId */ + 4 /* reserved */ + SECURITY_COOKIE_LEN /* securityCookie */;
    const NAME: &'static str = "TunnelCreateRequestPdu";

    /// Builds a tunnel-create request.
    pub fn new(request_id: u32, security_cookie: [u8; SECURITY_COOKIE_LEN]) -> Self {
        Self {
            request_id,
            security_cookie,
        }
    }
}

impl Encode for TunnelCreateRequestPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        TunnelHeader::encode(TunnelAction::CreateRequest, &[], Self::PAYLOAD_SIZE, dst)?;
        dst.write_u32(self.request_id);
        dst.write_u32(0);
        dst.write_slice(&self.security_cookie);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        TunnelHeader::encoded_size(&[]) + Self::PAYLOAD_SIZE
    }
}

impl<'de> Decode<'de> for TunnelCreateRequestPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let (header, payload_length) = TunnelHeader::decode(src)?;
        if header.action != TunnelAction::CreateRequest {
            return Err(invalid_field_err!("action", "expected tunnel create request"));
        }
        if !header.subheaders.is_empty() {
            return Err(invalid_field_err!(
                "headerLength",
                "must be four bytes for a tunnel create request"
            ));
        }
        if payload_length != Self::PAYLOAD_SIZE {
            return Err(invalid_field_err!(
                "payloadLength",
                "must be 24 bytes for a tunnel create request"
            ));
        }

        let request_id = src.read_u32();
        let reserved = src.read_u32();
        if reserved != 0 {
            return Err(invalid_field_err!("reserved", "must be zero"));
        }
        let security_cookie = src.read_array();

        Ok(Self {
            request_id,
            security_cookie,
        })
    }
}

/// Server response to an RDPEMT tunnel-create request.
///
/// Defined in [\[MS-RDPEMT\] 2.2.2.2].
///
/// [\[MS-RDPEMT\] 2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/d22b606c-32c4-4647-b356-86f75e23a22c
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct TunnelCreateResponsePdu {
    /// HRESULT returned by the server.
    pub hr_response: u32,
}

impl TunnelCreateResponsePdu {
    const PAYLOAD_SIZE: usize = 4 /* hrResponse */;
    const NAME: &'static str = "TunnelCreateResponsePdu";

    /// Exact `S_OK` response value.
    pub const S_OK: u32 = 0;

    /// Whether the server accepted the tunnel.
    pub fn is_success(&self) -> bool {
        self.hr_response == Self::S_OK
    }
}

impl Encode for TunnelCreateResponsePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        TunnelHeader::encode(TunnelAction::CreateResponse, &[], Self::PAYLOAD_SIZE, dst)?;
        dst.write_u32(self.hr_response);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        TunnelHeader::encoded_size(&[]) + Self::PAYLOAD_SIZE
    }
}

impl<'de> Decode<'de> for TunnelCreateResponsePdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let (header, payload_length) = TunnelHeader::decode(src)?;
        if header.action != TunnelAction::CreateResponse {
            return Err(invalid_field_err!("action", "expected tunnel create response"));
        }
        if !header.subheaders.is_empty() {
            return Err(invalid_field_err!(
                "headerLength",
                "must be four bytes for a tunnel create response"
            ));
        }
        if payload_length != Self::PAYLOAD_SIZE {
            return Err(invalid_field_err!(
                "payloadLength",
                "must be four bytes for a tunnel create response"
            ));
        }

        Ok(Self {
            hr_response: src.read_u32(),
        })
    }
}

/// Higher-layer data transported by an established RDPEMT tunnel.
///
/// Defined in [\[MS-RDPEMT\] 2.2.2.3].
///
/// [\[MS-RDPEMT\] 2.2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/d22b606c-32c4-4647-b356-86f75e23a22c
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct TunnelDataPdu {
    /// Optional autodetect subheaders.
    pub subheaders: Vec<TunnelSubheader>,
    /// Complete higher-layer message.
    pub data: Vec<u8>,
}

impl TunnelDataPdu {
    const NAME: &'static str = "TunnelDataPdu";

    /// Builds a tunnel data PDU without optional subheaders.
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            subheaders: Vec::new(),
            data,
        }
    }
}

impl Encode for TunnelDataPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        TunnelHeader::encode(TunnelAction::Data, &self.subheaders, self.data.len(), dst)?;
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        TunnelHeader::encoded_size(&self.subheaders) + self.data.len()
    }
}

impl<'de> Decode<'de> for TunnelDataPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let (header, payload_length) = TunnelHeader::decode(src)?;
        if header.action != TunnelAction::Data {
            return Err(invalid_field_err!("action", "expected tunnel data"));
        }

        Ok(Self {
            subheaders: header.subheaders,
            data: src.read_slice(payload_length).to_vec(),
        })
    }
}

/// Any complete RDPEMT tunnel PDU.
///
/// Defined in [\[MS-RDPEMT\] 2.2.2].
///
/// [\[MS-RDPEMT\] 2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemt/d22b606c-32c4-4647-b356-86f75e23a22c
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum TunnelPdu {
    /// Client tunnel-create request.
    CreateRequest(TunnelCreateRequestPdu),
    /// Server tunnel-create response.
    CreateResponse(TunnelCreateResponsePdu),
    /// Higher-layer data.
    Data(TunnelDataPdu),
}

impl Encode for TunnelPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        match self {
            Self::CreateRequest(pdu) => pdu.encode(dst),
            Self::CreateResponse(pdu) => pdu.encode(dst),
            Self::Data(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::CreateRequest(pdu) => pdu.name(),
            Self::CreateResponse(pdu) => pdu.name(),
            Self::Data(pdu) => pdu.name(),
        }
    }

    fn size(&self) -> usize {
        match self {
            Self::CreateRequest(pdu) => pdu.size(),
            Self::CreateResponse(pdu) => pdu.size(),
            Self::Data(pdu) => pdu.size(),
        }
    }
}

impl<'de> Decode<'de> for TunnelPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: TunnelHeader::FIXED_PART_SIZE);
        let action = TunnelAction::from_u8(src.remaining()[0] & 0x0F)
            .ok_or_else(|| invalid_field_err!("action", "unknown tunnel action"))?;

        match action {
            TunnelAction::CreateRequest => TunnelCreateRequestPdu::decode(src).map(Self::CreateRequest),
            TunnelAction::CreateResponse => TunnelCreateResponsePdu::decode(src).map(Self::CreateResponse),
            TunnelAction::Data => TunnelDataPdu::decode(src).map(Self::Data),
        }
    }
}

/// Requested transport protocol for multitransport bootstrapping.
///
/// Defined in [\[MS-RDPBCGR\] 2.2.15.1], `requestedProtocol` field.
///
/// [\[MS-RDPBCGR\] 2.2.15.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/de783158-8b01-4818-8fb0-62523a5b3490
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[repr(u16)]
pub enum RequestedProtocol {
    /// Reliable UDP transport (RDPEUDP2 + TLS).
    ///
    /// `INITIATE_REQUEST_PROTOCOL_UDPFECR`
    UdpFecR = 0x0001,
    /// Lossy UDP transport (RDPEUDP + DTLS, with forward error correction).
    ///
    /// `INITIATE_REQUEST_PROTOCOL_UDPFECL`
    UdpFecL = 0x0002,
}

impl RequestedProtocol {
    fn from_u16(val: u16) -> Option<Self> {
        match val {
            0x0001 => Some(Self::UdpFecR),
            0x0002 => Some(Self::UdpFecL),
            _ => None,
        }
    }

    #[expect(
        clippy::as_conversions,
        reason = "repr(u16) guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn as_u16(self) -> u16 {
        self as u16
    }
}

/// Server Initiate Multitransport Request PDU.
///
/// Sent by the server on the IO channel after licensing to bootstrap a
/// sideband UDP transport. The `request_id` and `security_cookie` are
/// echoed by the client in the tunnel creation request over the new
/// transport, binding the two connections together.
///
/// A server may send up to two of these — one for reliable and one for
/// lossy UDP.
///
/// Defined in [\[MS-RDPBCGR\] 2.2.15.1].
///
/// [\[MS-RDPBCGR\] 2.2.15.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/de783158-8b01-4818-8fb0-62523a5b3490
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MultitransportRequestPdu {
    pub security_header: BasicSecurityHeader,
    /// Unique ID correlating this request with the tunnel creation request.
    pub request_id: u32,
    /// Which transport protocol the server is requesting.
    pub requested_protocol: RequestedProtocol,
    /// 16-byte random cookie for transport binding validation.
    pub security_cookie: [u8; SECURITY_COOKIE_LEN],
}

impl MultitransportRequestPdu {
    const NAME: &'static str = "MultitransportRequestPdu";

    const FIXED_PART_SIZE: usize = BasicSecurityHeader::FIXED_PART_SIZE
        + 4 /* requestId */
        + 2 /* requestedProtocol */
        + 2 /* reserved */
        + SECURITY_COOKIE_LEN /* securityCookie */;
}

impl Encode for MultitransportRequestPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.security_header.encode(dst)?;
        dst.write_u32(self.request_id);
        dst.write_u16(self.requested_protocol.as_u16());
        write_padding!(dst, 2);
        dst.write_slice(&self.security_cookie);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for MultitransportRequestPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let security_header = BasicSecurityHeader::decode(src)?;

        // MS-RDPBCGR 2.2.15.1 requires the flags to *contain* SEC_TRANSPORT_REQ,
        // and 2.2.8.1.1.2.1 says SEC_RESET_SEQNO and SEC_IGNORE_SEQNO "MUST be
        // ignored", so they are masked off before comparing rather than treated
        // as a mismatch.
        //
        // What remains is compared for equality, which is stricter than the
        // spec requires and deliberately so: a successful decode is what tells
        // the connector that what arrived really is a request. The connector
        // narrows by MCS channel first, but the message channel also carries
        // auto-detect traffic, whose SEC_AUTODETECT_REQ/RSP a bare `contains`
        // check would happily accept as a multitransport request.
        let flags = security_header
            .flags
            .difference(BasicSecurityHeaderFlags::RESET_SEQNO | BasicSecurityHeaderFlags::IGNORE_SEQNO);
        if flags != BasicSecurityHeaderFlags::TRANSPORT_REQ {
            return Err(invalid_field_err!(
                "securityHeader",
                "expected securityHeader flags to contain SEC_TRANSPORT_REQ and no other PDU-type flag"
            ));
        }

        let request_id = src.read_u32();

        let protocol_raw = src.read_u16();
        let requested_protocol = RequestedProtocol::from_u16(protocol_raw)
            .ok_or_else(|| invalid_field_err!("requestedProtocol", "unknown protocol value"))?;

        read_padding!(src, 2);

        let security_cookie: [u8; SECURITY_COOKIE_LEN] = src.read_array();

        Ok(Self {
            security_header,
            request_id,
            requested_protocol,
            security_cookie,
        })
    }
}

/// Client Initiate Multitransport Response PDU.
///
/// Sent by the client on the IO channel after the UDP transport is
/// established (or has failed). The `request_id` must match the
/// corresponding server request.
///
/// Defined in [\[MS-RDPBCGR\] 2.2.15.2].
///
/// [\[MS-RDPBCGR\] 2.2.15.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/44044233-e498-46f8-8e16-1ffa595a8e8b
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct MultitransportResponsePdu {
    pub security_header: BasicSecurityHeader,
    /// Request ID matching the server's Initiate Multitransport Request.
    pub request_id: u32,
    /// HRESULT indicating success or failure of the transport setup.
    pub hr_response: u32,
}

impl MultitransportResponsePdu {
    const NAME: &'static str = "MultitransportResponsePdu";

    const FIXED_PART_SIZE: usize = BasicSecurityHeader::FIXED_PART_SIZE
        + 4 /* requestId */
        + 4 /* hrResponse */;

    /// `S_OK` — multitransport connection established.
    ///
    /// Per [\[MS-RDPBCGR\] 2.2.15.2], this MUST only be sent to servers that
    /// advertised `SOFTSYNC_TCP_TO_UDP` in the GCC `MultiTransportChannelData`.
    ///
    /// [\[MS-RDPBCGR\] 2.2.15.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/44044233-e498-46f8-8e16-1ffa595a8e8b
    pub const S_OK: u32 = 0x0000_0000;

    /// `E_ABORT` — client was unable to establish the multitransport connection.
    pub const E_ABORT: u32 = 0x8000_4004;

    /// Create a success response for the given request ID.
    pub fn success(request_id: u32) -> Self {
        Self {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_RSP,
            },
            request_id,
            hr_response: Self::S_OK,
        }
    }

    /// Create a failure response for the given request ID.
    pub fn abort(request_id: u32) -> Self {
        Self {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_RSP,
            },
            request_id,
            hr_response: Self::E_ABORT,
        }
    }

    /// Whether this response indicates success.
    pub fn is_success(&self) -> bool {
        self.hr_response == Self::S_OK
    }
}

impl Encode for MultitransportResponsePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.security_header.encode(dst)?;
        dst.write_u32(self.request_id);
        dst.write_u32(self.hr_response);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for MultitransportResponsePdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let security_header = BasicSecurityHeader::decode(src)?;

        // MS-RDPBCGR 2.2.15.2 requires the flags to *contain* SEC_TRANSPORT_RSP,
        // and the ignorable sequence-number flags are masked off first, exactly as
        // on the request side; see the note on `MultitransportRequestPdu::decode`
        // for why what remains is then compared for equality.
        let flags = security_header
            .flags
            .difference(BasicSecurityHeaderFlags::RESET_SEQNO | BasicSecurityHeaderFlags::IGNORE_SEQNO);
        if flags != BasicSecurityHeaderFlags::TRANSPORT_RSP {
            return Err(invalid_field_err!(
                "securityHeader",
                "expected securityHeader flags to contain SEC_TRANSPORT_RSP and no other PDU-type flag"
            ));
        }

        let request_id = src.read_u32();
        let hr_response = src.read_u32();

        Ok(Self {
            security_header,
            request_id,
            hr_response,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST_WIRE: &[u8] = &[
        // BasicSecurityHeader (4 bytes)
        0x02, 0x00, // flags = TRANSPORT_REQ (0x0002)
        0x00, 0x00, // flagsHi = 0
        // Payload (24 bytes)
        0x2A, 0x00, 0x00, 0x00, // requestId = 42
        0x01, 0x00, // requestedProtocol = UdpFecR (0x0001)
        0x00, 0x00, // reserved
        0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, // securityCookie
        0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB,
    ];

    const RESPONSE_SUCCESS_WIRE: &[u8] = &[
        // BasicSecurityHeader (4 bytes)
        0x04, 0x00, // flags = TRANSPORT_RSP (0x0004)
        0x00, 0x00, // flagsHi = 0
        // Payload (8 bytes)
        0x2A, 0x00, 0x00, 0x00, // requestId = 42
        0x00, 0x00, 0x00, 0x00, // hrResponse = S_OK
    ];

    const RESPONSE_ABORT_WIRE: &[u8] = &[
        // BasicSecurityHeader (4 bytes)
        0x04, 0x00, // flags = TRANSPORT_RSP (0x0004)
        0x00, 0x00, // flagsHi = 0
        // Payload (8 bytes)
        0x07, 0x00, 0x00, 0x00, // requestId = 7
        0x04, 0x40, 0x00, 0x80, // hrResponse = E_ABORT (0x80004004)
    ];

    #[test]
    fn decode_request() {
        let pdu = ironrdp_core::decode::<MultitransportRequestPdu>(REQUEST_WIRE).unwrap();
        assert_eq!(pdu.request_id, 42);
        assert_eq!(pdu.requested_protocol, RequestedProtocol::UdpFecR);
        assert_eq!(pdu.security_cookie, [0xAB; 16]);
    }

    #[test]
    fn encode_request() {
        let pdu = MultitransportRequestPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
            },
            request_id: 42,
            requested_protocol: RequestedProtocol::UdpFecR,
            security_cookie: [0xAB; 16],
        };
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), REQUEST_WIRE);
    }

    #[test]
    fn request_round_trip() {
        let original = MultitransportRequestPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
            },
            request_id: 0xDEAD_BEEF,
            requested_protocol: RequestedProtocol::UdpFecL,
            security_cookie: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        };
        let encoded = ironrdp_core::encode_vec(&original).unwrap();
        let decoded = ironrdp_core::decode::<MultitransportRequestPdu>(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn request_size() {
        assert_eq!(MultitransportRequestPdu::FIXED_PART_SIZE, 28);
    }

    #[test]
    fn decode_response_success() {
        let pdu = ironrdp_core::decode::<MultitransportResponsePdu>(RESPONSE_SUCCESS_WIRE).unwrap();
        assert_eq!(pdu.request_id, 42);
        assert!(pdu.is_success());
    }

    #[test]
    fn decode_response_abort() {
        let pdu = ironrdp_core::decode::<MultitransportResponsePdu>(RESPONSE_ABORT_WIRE).unwrap();
        assert_eq!(pdu.request_id, 7);
        assert_eq!(pdu.hr_response, MultitransportResponsePdu::E_ABORT);
        assert!(!pdu.is_success());
    }

    #[test]
    fn encode_response_success() {
        let pdu = MultitransportResponsePdu::success(42);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), RESPONSE_SUCCESS_WIRE);
    }

    #[test]
    fn encode_response_abort() {
        let pdu = MultitransportResponsePdu::abort(7);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), RESPONSE_ABORT_WIRE);
    }

    #[test]
    fn response_round_trip() {
        let original = MultitransportResponsePdu::success(0xCAFE_BABE);
        let encoded = ironrdp_core::encode_vec(&original).unwrap();
        let decoded = ironrdp_core::decode::<MultitransportResponsePdu>(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn response_size() {
        assert_eq!(MultitransportResponsePdu::FIXED_PART_SIZE, 12);
    }

    #[test]
    fn decode_request_wrong_flags() {
        let bad_wire: &[u8] = &[
            0x04, 0x00, // flags = TRANSPORT_RSP (wrong for request)
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(ironrdp_core::decode::<MultitransportRequestPdu>(bad_wire).is_err());
    }

    /// MS-RDPBCGR 2.2.8.1.1.2.1 says SEC_RESET_SEQNO and SEC_IGNORE_SEQNO "MUST
    /// be ignored", and the spec's own worked examples carry them alongside other
    /// flags. Rejecting a PDU that sets them would abort the connection over bits
    /// the peer is told to disregard.
    ///
    /// Both directions are covered because the two decoders are symmetric: 2.2.15.1
    /// and 2.2.15.2 each say the flags MUST *contain* their respective discriminator.
    #[test]
    fn decode_ignores_the_seqno_flags_in_both_directions() {
        const RESET_SEQNO: u16 = 0x0010;
        const IGNORE_SEQNO: u16 = 0x0020;

        for (label, base) in [("request", 0x0002u16), ("response", 0x0004)] {
            for (variant, extra) in [
                ("SEC_RESET_SEQNO", RESET_SEQNO),
                ("SEC_IGNORE_SEQNO", IGNORE_SEQNO),
                ("both seqno flags", RESET_SEQNO | IGNORE_SEQNO),
            ] {
                let flags = base | extra;

                if base == 0x0002 {
                    let mut wire = REQUEST_WIRE.to_vec();
                    wire[0..2].copy_from_slice(&flags.to_le_bytes());
                    let pdu = ironrdp_core::decode::<MultitransportRequestPdu>(&wire)
                        .unwrap_or_else(|e| panic!("{label} with {variant} must decode: {e}"));
                    assert_eq!(pdu.request_id, 42, "{label} with {variant}");
                } else {
                    let mut wire = RESPONSE_SUCCESS_WIRE.to_vec();
                    wire[0..2].copy_from_slice(&flags.to_le_bytes());
                    let pdu = ironrdp_core::decode::<MultitransportResponsePdu>(&wire)
                        .unwrap_or_else(|e| panic!("{label} with {variant} must decode: {e}"));
                    assert_eq!(pdu.request_id, 42, "{label} with {variant}");
                }
            }
        }
    }

    /// The equality check past the ignorable flags is what keeps other traffic on
    /// the shared message channel from decoding as multitransport. Auto-detect is
    /// the case that actually shares the channel.
    #[test]
    fn decode_rejects_another_pdu_type_alongside_the_discriminator_in_both_directions() {
        const AUTODETECT_REQ: u16 = 0x1000;

        let mut request = REQUEST_WIRE.to_vec();
        request[0..2].copy_from_slice(&(0x0002u16 | AUTODETECT_REQ).to_le_bytes());
        assert!(ironrdp_core::decode::<MultitransportRequestPdu>(&request).is_err());

        let mut response = RESPONSE_SUCCESS_WIRE.to_vec();
        response[0..2].copy_from_slice(&(0x0004u16 | AUTODETECT_REQ).to_le_bytes());
        assert!(ironrdp_core::decode::<MultitransportResponsePdu>(&response).is_err());
    }

    #[test]
    fn decode_request_unknown_protocol() {
        let bad_wire: &[u8] = &[
            0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xFF, 0x00, // unknown protocol
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(ironrdp_core::decode::<MultitransportRequestPdu>(bad_wire).is_err());
    }

    #[test]
    fn tunnel_create_request_round_trip() {
        let pdu = TunnelCreateRequestPdu::new(7, [0xAB; SECURITY_COOKIE_LEN]);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(
            encoded.as_slice(),
            &[
                0x00, // action = create request, flags = 0
                0x18, 0x00, // payloadLength = 24
                0x04, // headerLength = 4
                0x07, 0x00, 0x00, 0x00, // requestId
                0x00, 0x00, 0x00, 0x00, // reserved
                0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, // securityCookie
                0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB,
            ]
        );
        let decoded = ironrdp_core::decode::<TunnelCreateRequestPdu>(&encoded).unwrap();
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn tunnel_create_pdus_reject_subheaders() {
        let request = [
            0x00, // action = create request, flags = 0
            0x18, 0x00, // payloadLength = 24
            0x06, // headerLength = 6
            0x02, 0x01, // subheader
            0x07, 0x00, 0x00, 0x00, // requestId
            0x00, 0x00, 0x00, 0x00, // reserved
            0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, // securityCookie
            0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB, 0xAB,
        ];
        assert!(ironrdp_core::decode::<TunnelCreateRequestPdu>(&request).is_err());

        let response = [
            0x01, // action = create response, flags = 0
            0x04, 0x00, // payloadLength = 4
            0x06, // headerLength = 6
            0x02, 0x01, // subheader
            0x00, 0x00, 0x00, 0x00, // hrResponse
        ];
        assert!(ironrdp_core::decode::<TunnelCreateResponsePdu>(&response).is_err());
    }

    #[test]
    fn tunnel_pdu_preserves_subheaders_and_data() {
        let pdu = TunnelPdu::Data(TunnelDataPdu {
            subheaders: vec![TunnelSubheader {
                ty: 1,
                data: vec![0xAA, 0xBB],
            }],
            data: vec![1, 2, 3],
        });
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(
            encoded.as_slice(),
            &[
                0x02, // action = data, flags = 0
                0x03, 0x00, // payloadLength = 3
                0x08, // headerLength = 8
                0x04, 0x01, 0xAA, 0xBB, // subheader
                0x01, 0x02, 0x03, // higher-layer data
            ]
        );
        let decoded = ironrdp_core::decode::<TunnelPdu>(&encoded).unwrap();
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn tunnel_decoder_leaves_the_next_pdu_in_the_cursor() {
        let first = ironrdp_core::encode_vec(&TunnelCreateResponsePdu {
            hr_response: TunnelCreateResponsePdu::S_OK,
        })
        .unwrap();
        let second = ironrdp_core::encode_vec(&TunnelCreateResponsePdu {
            hr_response: 0x8000_4004,
        })
        .unwrap();
        let mut wire = first;
        wire.extend_from_slice(&second);

        let mut cursor = ReadCursor::new(&wire);
        assert_eq!(
            TunnelCreateResponsePdu::decode(&mut cursor).unwrap().hr_response,
            TunnelCreateResponsePdu::S_OK
        );
        assert_eq!(
            TunnelCreateResponsePdu::decode(&mut cursor).unwrap().hr_response,
            0x8000_4004
        );
        assert!(cursor.is_empty());
    }

    #[test]
    fn tunnel_decoder_rejects_truncated_payloads() {
        let wire = [
            0x01, // action = create response, flags = 0
            0x04, 0x00, // payloadLength = 4
            0x04, // headerLength = 4
            0x00, 0x00, 0x00, // incomplete hrResponse
        ];
        assert!(ironrdp_core::decode::<TunnelCreateResponsePdu>(&wire).is_err());
    }

    #[test]
    fn tunnel_encoders_return_errors_for_small_buffers() {
        let request = TunnelCreateRequestPdu::new(7, [0xAB; SECURITY_COOKIE_LEN]);
        let response = TunnelCreateResponsePdu {
            hr_response: TunnelCreateResponsePdu::S_OK,
        };
        let data = TunnelDataPdu::new(vec![1]);

        assert!(ironrdp_core::encode(&request, &mut [0; 1]).is_err());
        assert!(ironrdp_core::encode(&response, &mut [0; 1]).is_err());
        assert!(ironrdp_core::encode(&data, &mut [0; 1]).is_err());
    }

    #[test]
    fn tunnel_header_rejects_invalid_lengths_and_flags() {
        let invalid_header_length = [0x02, 0x00, 0x00, 0x03];
        assert!(ironrdp_core::decode::<TunnelDataPdu>(&invalid_header_length).is_err());

        let invalid_flags = [0x12, 0x00, 0x00, 0x04];
        assert!(ironrdp_core::decode::<TunnelDataPdu>(&invalid_flags).is_err());

        let invalid_payload_length = [0x02, 0x01, 0x00, 0x04];
        assert!(ironrdp_core::decode::<TunnelDataPdu>(&invalid_payload_length).is_err());
    }
}
