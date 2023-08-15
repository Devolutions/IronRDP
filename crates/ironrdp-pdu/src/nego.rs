//! PDUs used during the Connection Initiation stage

use bitflags::bitflags;
use tap::prelude::*;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::tpdu::{TpduCode, TpduHeader};
use crate::tpkt::TpktHeader;
use crate::x224::X224Pdu;
use crate::{Pdu as _, PduError, PduErrorExt as _, PduResult};

bitflags! {
    /// A 32-bit, unsigned integer that contains flags indicating the supported
    /// security protocols.
    /// The client and server agree on it during the Connection Initiation phase.
    ///
    /// # MSDN
    ///
    /// * [RDP Negotiation Request (RDP_NEG_REQ)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/902b090b-9cb3-4efc-92bf-ee13373371e3)
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct SecurityProtocol: u32 {
        const RDP = 0x0000_0000;
        const SSL = 0x0000_0001;
        const HYBRID = 0x0000_0002;
        const RDSTLS = 0x0000_0004;
        const HYBRID_EX = 0x0000_0008;
        const RDSAAD = 0x0000_0010;
    }
}

bitflags! {
    /// Holds the negotiation protocol flags of the *request* message.
    ///
    /// # MSDN
    ///
    /// * [RDP Negotiation Request (RDP_NEG_REQ)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/902b090b-9cb3-4efc-92bf-ee13373371e3)
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct RequestFlags: u8 {
        const RESTRICTED_ADMIN_MODE_REQUIRED = 0x01;
        const REDIRECTED_AUTHENTICATION_MODE_REQUIRED = 0x02;
        const CORRELATION_INFO_PRESENT = 0x08;
    }
}

bitflags! {
    /// Holds the negotiation protocol flags of the *response* message.
    ///
    /// # MSDN
    ///
    /// * [RDP Negotiation Response (RDP_NEG_RSP)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/b2975bdc-6d56-49ee-9c57-f2ff3a0b6817)
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ResponseFlags: u8 {
        const EXTENDED_CLIENT_DATA_SUPPORTED = 0x01;
        const DYNVC_GFX_PROTOCOL_SUPPORTED = 0x02;
        const RDP_NEG_RSP_RESERVED = 0x04;
        const RESTRICTED_ADMIN_MODE_SUPPORTED = 0x08;
        const REDIRECTED_AUTHENTICATION_MODE_SUPPORTED = 0x10;
    }
}

/// The type of the negotiation error. May be contained in ResponseData.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FailureCode(u32);

impl FailureCode {
    pub const SSL_REQUIRED_BY_SERVER: Self = Self(1);
    pub const SSL_NOT_ALLOWED_BY_SERVER: Self = Self(2);
    pub const SSL_CERT_NOT_ON_SERVER: Self = Self(3);
    pub const INCONSISTENT_FLAGS: Self = Self(4);
    pub const HYBRID_REQUIRED_BY_SERVER: Self = Self(5);
    /// Used when the failure caused by ResponseFailure.
    pub const SSL_WITH_USER_AUTH_REQUIRED_BY_SERVER: Self = Self(6);
}

impl From<u32> for FailureCode {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<FailureCode> for u32 {
    fn from(value: FailureCode) -> Self {
        value.0
    }
}

/// The kind of the negotiation request message.
///
/// # MSDN
///
/// * [Client X.224 Connection Request PDU](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/18a27ef9-6f9a-4501-b000-94b1fe3c2c10)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NegoRequestData {
    RoutingToken(RoutingToken),
    Cookie(Cookie),
}

impl NegoRequestData {
    pub fn routing_token(value: String) -> Self {
        Self::RoutingToken(RoutingToken(value))
    }

    pub fn cookie(value: String) -> Self {
        Self::Cookie(Cookie(value))
    }

    pub fn read(src: &mut ReadCursor<'_>) -> PduResult<Option<Self>> {
        match RoutingToken::read(src)? {
            Some(token) => Ok(Some(Self::RoutingToken(token))),
            None => Cookie::read(src)?.map(Self::Cookie).pipe(Ok),
        }
    }

    pub fn write(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            NegoRequestData::RoutingToken(token) => token.write(dst),
            NegoRequestData::Cookie(cookie) => cookie.write(dst),
        }
    }

    pub fn size(&self) -> usize {
        match self {
            NegoRequestData::RoutingToken(token) => token.size(),
            NegoRequestData::Cookie(cookie) => cookie.size(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie(pub String);

impl Cookie {
    const PREFIX: &str = "Cookie: mstshash=";

    pub fn read(src: &mut ReadCursor<'_>) -> PduResult<Option<Self>> {
        read_nego_data(src, "Cookie", Self::PREFIX)?.map(Self).pipe(Ok)
    }

    pub fn write(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_nego_data(dst, "Cookie", Self::PREFIX, &self.0)
    }

    pub fn size(&self) -> usize {
        Self::PREFIX.len() + self.0.len() + 2
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingToken(pub String);

impl RoutingToken {
    const PREFIX: &str = "Cookie: msts=";

    pub fn read(src: &mut ReadCursor<'_>) -> PduResult<Option<Self>> {
        read_nego_data(src, "RoutingToken", Self::PREFIX)?.map(Self).pipe(Ok)
    }

    pub fn write(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_nego_data(dst, "RoutingToken", Self::PREFIX, &self.0)
    }

    pub fn size(&self) -> usize {
        Self::PREFIX.len() + self.0.len() + 2
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct NegoMsgType(u8);

impl NegoMsgType {
    const REQUEST: Self = Self(0x01);
    const RESPONSE: Self = Self(0x02);
    const FAILURE: Self = Self(0x03);
}

impl From<u8> for NegoMsgType {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<NegoMsgType> for u8 {
    fn from(value: NegoMsgType) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRequest {
    pub nego_data: Option<NegoRequestData>,
    pub flags: RequestFlags,
    pub protocol: SecurityProtocol,
}

impl_pdu_pod!(ConnectionRequest);

impl ConnectionRequest {
    const RDP_NEG_REQ_SIZE: u16 = 8;
}

impl<'de> X224Pdu<'de> for ConnectionRequest {
    const X224_NAME: &'static str = "Client X.224 Connection Request";

    const TPDU_CODE: TpduCode = TpduCode::CONNECTION_REQUEST;

    fn x224_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        if let Some(nego_data) = &self.nego_data {
            nego_data.write(dst)?;
        }

        if self.protocol != SecurityProtocol::RDP {
            dst.write_u8(u8::from(NegoMsgType::REQUEST));
            dst.write_u8(self.flags.bits());
            dst.write_u16(Self::RDP_NEG_REQ_SIZE);
            dst.write_u32(self.protocol.bits());
        }

        Ok(())
    }

    fn x224_body_decode(src: &mut ReadCursor<'de>, _: &TpktHeader, tpdu: &TpduHeader) -> PduResult<Self> {
        let variable_part_size = tpdu.variable_part_size();

        ensure_size!(in: src, size: variable_part_size);

        let nego_data = NegoRequestData::read(src)?;

        let Some(variable_part_rest_size) =
            variable_part_size.checked_sub(nego_data.as_ref().map(|data| data.size()).unwrap_or(0))
        else {
            return Err(PduError::invalid_message(
                Self::NAME,
                "TPDU header variable part",
                "advertised size too small",
            ));
        };

        if variable_part_rest_size >= usize::from(Self::RDP_NEG_REQ_SIZE) {
            let msg_type = NegoMsgType::from(src.read_u8());

            if msg_type != NegoMsgType::REQUEST {
                return Err(PduError::unexpected_message_type(Self::NAME, u8::from(msg_type)));
            }

            let flags = RequestFlags::from_bits_truncate(src.read_u8());

            if flags.contains(RequestFlags::CORRELATION_INFO_PRESENT) {
                // TODO: support for RDP_NEG_CORRELATION_INFO
                return Err(PduError::invalid_message(
                    Self::NAME,
                    "flags",
                    "CORRECTION_INFO_PRESENT flag is set, but not supported by IronRDP",
                ));
            }

            let _length = src.read_u16();

            let protocol = SecurityProtocol::from_bits_truncate(src.read_u32());

            Ok(Self {
                nego_data,
                flags,
                protocol,
            })
        } else {
            Ok(Self {
                nego_data,
                flags: RequestFlags::empty(),
                protocol: SecurityProtocol::RDP,
            })
        }
    }

    fn tpdu_header_variable_part_size(&self) -> usize {
        let optional_nego_data_size = self.nego_data.as_ref().map(|data| data.size()).unwrap_or(0);

        let rdp_neg_req_size = if self.protocol == SecurityProtocol::RDP {
            0
        } else {
            usize::from(Self::RDP_NEG_REQ_SIZE)
        };

        optional_nego_data_size + rdp_neg_req_size
    }

    fn tpdu_user_data_size(&self) -> usize {
        0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionConfirm {
    Response {
        flags: ResponseFlags,
        protocol: SecurityProtocol,
    },
    Failure {
        code: FailureCode,
    },
}

impl_pdu_pod!(ConnectionConfirm);

impl ConnectionConfirm {
    const RDP_NEG_RSP: u16 = 8;

    const RDP_NEG_FAILURE: u16 = 8;
}

impl<'de> X224Pdu<'de> for ConnectionConfirm {
    const X224_NAME: &'static str = "Server X.224 Connection Confirm";

    const TPDU_CODE: TpduCode = TpduCode::CONNECTION_CONFIRM;

    fn x224_body_encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            ConnectionConfirm::Response { flags, protocol } => {
                dst.write_u8(u8::from(NegoMsgType::RESPONSE));
                dst.write_u8(flags.bits());
                dst.write_u16(Self::RDP_NEG_RSP);
                dst.write_u32(protocol.bits());
            }
            ConnectionConfirm::Failure { code } => {
                dst.write_u8(u8::from(NegoMsgType::FAILURE));
                dst.write_u8(0);
                dst.write_u16(Self::RDP_NEG_RSP);
                dst.write_u32(u32::from(*code));
            }
        }

        Ok(())
    }

    fn x224_body_decode(src: &mut ReadCursor<'de>, _: &TpktHeader, tpdu: &TpduHeader) -> PduResult<Self> {
        let variable_part_size = tpdu.variable_part_size();

        ensure_size!(in: src, size: variable_part_size);

        if variable_part_size > 0 {
            ensure_size!(in: src, size: 8); // message type (1) + flags (1) + length (2) + code / protocol (4)

            match NegoMsgType::from(src.read_u8()) {
                NegoMsgType::RESPONSE => {
                    let flags = ResponseFlags::from_bits_truncate(src.read_u8());
                    let _length = src.read_u16();
                    let protocol = SecurityProtocol::from_bits_truncate(src.read_u32());

                    Ok(Self::Response { flags, protocol })
                }
                NegoMsgType::FAILURE => {
                    let _flags = src.read_u8();
                    let _length = src.read_u16();
                    let code = FailureCode::from(src.read_u32());

                    Ok(Self::Failure { code })
                }
                unexpected => Err(PduError::unexpected_message_type(Self::X224_NAME, u8::from(unexpected))),
            }
        } else {
            Ok(Self::Response {
                flags: ResponseFlags::empty(),
                protocol: SecurityProtocol::RDP,
            })
        }
    }

    fn tpdu_header_variable_part_size(&self) -> usize {
        match self {
            ConnectionConfirm::Response { .. } => usize::from(Self::RDP_NEG_RSP),
            ConnectionConfirm::Failure { .. } => usize::from(Self::RDP_NEG_FAILURE),
        }
    }

    fn tpdu_user_data_size(&self) -> usize {
        0
    }
}

fn read_nego_data(src: &mut ReadCursor<'_>, ctx: &'static str, prefix: &str) -> PduResult<Option<String>> {
    if src.len() < prefix.len() + 2 {
        return Ok(None);
    }

    if src.peek_slice(prefix.len()) != prefix.as_bytes() {
        return Ok(None);
    }

    src.advance(prefix.len());

    let identifier_start = src.pos();

    while src.peek_u16() != 0x0A0D {
        src.advance(1);
        ensure_size!(ctx: ctx, in: src, size: 2);
    }

    let identifier_end = src.pos();

    src.advance(2);

    let data = core::str::from_utf8(&src.inner()[identifier_start..identifier_end])
        .map_err(|_| PduError::invalid_message(ctx, "identifier", "not valid UTF-8"))?
        .to_owned();

    Ok(Some(data))
}

fn write_nego_data(dst: &mut WriteCursor<'_>, ctx: &'static str, prefix: &str, value: &str) -> PduResult<()> {
    ensure_size!(ctx: ctx, in: dst, size: prefix.len() + value.len() + 2);

    dst.write_slice(prefix.as_bytes());
    dst.write_slice(value.as_bytes());
    dst.write_u16(0x0A0D);

    Ok(())
}
