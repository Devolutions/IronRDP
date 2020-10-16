#[cfg(test)]
mod tests;

use std::io::{self, prelude::*};

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{
    impl_from_error,
    x224::{TpktHeader, X224TPDUType, TPDU_REQUEST_LENGTH, TPKT_HEADER_LENGTH},
    PduParsing,
};

const COOKIE_PREFIX: &str = "Cookie: mstshash=";
const ROUTING_TOKEN_PREFIX: &str = "Cookie: msts=";

const RDP_NEG_DATA_LENGTH: u16 = 8;
const CR_LF_SEQ_LENGTH: usize = 2;

bitflags! {
    /// The communication protocol which the client and server agree to transfer
    /// data on during the X.224 phase.
    ///
    /// # MSDN
    ///
    /// * [RDP Negotiation Request (RDP_NEG_REQ)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/902b090b-9cb3-4efc-92bf-ee13373371e3)
    pub struct SecurityProtocol: u32 {
        const RDP = 0;
        const SSL = 1;
        const HYBRID = 2;
        const RDSTLS = 4;
        const HYBRID_EX = 8;
    }
}

bitflags! {
    /// Holds the negotiation protocol flags of the *request* message.
    ///
    /// # MSDN
    ///
    /// * [RDP Negotiation Request (RDP_NEG_REQ)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/902b090b-9cb3-4efc-92bf-ee13373371e3)
    #[derive(Default)]
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
    #[derive(Default)]
    pub struct ResponseFlags: u8 {
        const EXTENDED_CLIENT_DATA_SUPPORTED = 0x01;
        const DYNVC_GFX_PROTOCOL_SUPPORTED = 0x02;
        const RDP_NEG_RSP_RESERVED = 0x04;
        const RESTRICTED_ADMIN_MODE_SUPPORTED = 0x08;
        const REDIRECTED_AUTHENTICATION_MODE_SUPPORTED = 0x10;
    }
}

/// The type of the negotiation error. May be contained in
/// [`ResponseData`](enum.ResponseData.html).
#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum FailureCode {
    SSLRequiredByServer = 1,
    SSLNotAllowedByServer = 2,
    SSLCertNotOnServer = 3,
    InconsistentFlags = 4,
    HybridRequiredByServer = 5,
    /// Used when the failure caused by [`ResponseFailure`](struct.ResponseFailure.html).
    SSLWithUserAuthRequiredByServer = 6,
}

/// The kind of the negotiation request message, including the message as a
/// [`String`](https://doc.rust-lang.org/std/string/struct.String.html).
///
/// # MSDN
///
/// * [Client X.224 Connection Request PDU](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/18a27ef9-6f9a-4501-b000-94b1fe3c2c10)
#[derive(Debug, Clone, PartialEq)]
pub enum NegoData {
    RoutingToken(String),
    Cookie(String),
}

///  The type of the error that may result from a negotiation process.
#[derive(Debug, Fail)]
pub enum NegotiationError {
    /// Corresponds for an I/O error that may occur during a negotiation process
    /// (invalid response code, invalid security protocol code, etc.)
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    /// May indicate about a negotiation error recieved from a server.
    #[fail(display = "Received negotiation error from server, code={:?}", 0)]
    ResponseFailure(FailureCode),
    #[fail(display = "Invalid tpkt header version")]
    TpktVersionError,
}

impl_from_error!(io::Error, NegotiationError, NegotiationError::IOError);

impl From<NegotiationError> for io::Error {
    fn from(e: NegotiationError) -> io::Error {
        io::Error::new(io::ErrorKind::Other, format!("Negotiation error: {}", e))
    }
}

#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
enum Message {
    Request = 1,
    Response = 2,
    Failure = 3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub nego_data: Option<NegoData>,
    pub flags: RequestFlags,
    pub protocol: SecurityProtocol,
    pub src_ref: u16,
}

impl PduParsing for Request {
    type Error = NegotiationError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _tpkt = TpktHeader::from_buffer(&mut stream)?;

        crate::x224::read_and_check_tpdu_header(&mut stream, X224TPDUType::ConnectionRequest)?;

        let _dst_ref = stream.read_u16::<LittleEndian>()?;
        let src_ref = stream.read_u16::<LittleEndian>()?;

        read_and_check_class(&mut stream, 0)?;

        let mut stream = io::BufReader::new(stream);
        let mut buffer = Vec::new();

        stream.read_to_end(&mut buffer)?;
        let mut stream = buffer.as_slice();

        let nego_data = if let Some((nego_data, read_len)) = read_nego_data(stream) {
            stream.consume(read_len);

            Some(nego_data)
        } else {
            None
        };

        if stream.len() >= RDP_NEG_DATA_LENGTH as usize {
            let neg_req = Message::from_u8(stream.read_u8()?).ok_or_else(|| {
                NegotiationError::IOError(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid negotiation request code",
                ))
            })?;
            if neg_req != Message::Request {
                return Err(NegotiationError::IOError(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid negotiation request code",
                )));
            }

            let flags = RequestFlags::from_bits_truncate(stream.read_u8()?);
            let _length = stream.read_u16::<LittleEndian>()?;
            let protocol = SecurityProtocol::from_bits_truncate(stream.read_u32::<LittleEndian>()?);

            Ok(Self {
                nego_data,
                flags,
                protocol,
                src_ref,
            })
        } else {
            Ok(Self {
                nego_data,
                flags: RequestFlags::empty(),
                protocol: SecurityProtocol::RDP,
                src_ref,
            })
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        TpktHeader::new(self.buffer_length()).to_buffer(&mut stream)?;

        let tpdu_length = self.buffer_length() - TPKT_HEADER_LENGTH - 1;
        stream.write_u8(tpdu_length as u8)?;

        stream.write_u8(X224TPDUType::ConnectionRequest.to_u8().unwrap())?;
        stream.write_u16::<LittleEndian>(0)?; // dst_ref
        stream.write_u16::<LittleEndian>(self.src_ref)?;
        stream.write_u8(0)?; // class

        match &self.nego_data {
            Some(NegoData::Cookie(s)) => writeln!(&mut stream, "{}{}\r", COOKIE_PREFIX, s)?,
            Some(NegoData::RoutingToken(s)) => {
                writeln!(&mut stream, "{}{}\r", ROUTING_TOKEN_PREFIX, s)?
            }
            None => (),
        }

        if self.protocol.bits() > SecurityProtocol::RDP.bits() {
            stream.write_u8(Message::Request.to_u8().unwrap())?;
            stream.write_u8(self.flags.bits())?;
            stream.write_u16::<LittleEndian>(RDP_NEG_DATA_LENGTH as u16)?;
            stream.write_u32::<LittleEndian>(self.protocol.bits())?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        TPDU_REQUEST_LENGTH
            + match &self.nego_data {
                Some(NegoData::Cookie(s)) => s.len() + COOKIE_PREFIX.len() + CR_LF_SEQ_LENGTH,
                Some(NegoData::RoutingToken(s)) => {
                    s.len() + ROUTING_TOKEN_PREFIX.len() + CR_LF_SEQ_LENGTH
                }
                None => 0,
            }
            + if self.protocol.bits() > SecurityProtocol::RDP.bits() {
                usize::from(RDP_NEG_DATA_LENGTH)
            } else {
                0
            }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseData {
    Response {
        flags: ResponseFlags,
        protocol: SecurityProtocol,
    },
    Failure {
        code: FailureCode,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub response: Option<ResponseData>,
    pub dst_ref: u16,
    pub src_ref: u16,
}

impl PduParsing for Response {
    type Error = NegotiationError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let _tpkt = TpktHeader::from_buffer(&mut stream)?;

        crate::x224::read_and_check_tpdu_header(&mut stream, X224TPDUType::ConnectionConfirm)?;

        let dst_ref = stream.read_u16::<LittleEndian>()?;
        let src_ref = stream.read_u16::<LittleEndian>()?;

        read_and_check_class(&mut stream, 0)?;

        let neg_resp = Message::from_u8(stream.read_u8()?).ok_or_else(|| {
            NegotiationError::IOError(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid negotiation response code",
            ))
        })?;
        let flags = ResponseFlags::from_bits_truncate(stream.read_u8()?);
        let _length = stream.read_u16::<LittleEndian>()?;

        match neg_resp {
            Message::Response => {
                let protocol =
                    SecurityProtocol::from_bits_truncate(stream.read_u32::<LittleEndian>()?);

                Ok(Self {
                    response: Some(ResponseData::Response { flags, protocol }),
                    dst_ref,
                    src_ref,
                })
            }
            Message::Failure => {
                let error =
                    FailureCode::from_u32(stream.read_u32::<LittleEndian>()?).ok_or_else(|| {
                        NegotiationError::IOError(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid negotiation failure code",
                        ))
                    })?;

                Err(NegotiationError::ResponseFailure(error))
            }
            _ => Err(NegotiationError::IOError(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid security protocol code",
            ))),
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        TpktHeader::new(self.buffer_length()).to_buffer(&mut stream)?;

        let tpdu_length = self.buffer_length() - TPKT_HEADER_LENGTH - 1;
        stream.write_u8(tpdu_length as u8)?;

        stream.write_u8(X224TPDUType::ConnectionConfirm.to_u8().unwrap())?;
        stream.write_u16::<LittleEndian>(self.dst_ref)?;
        stream.write_u16::<LittleEndian>(self.src_ref)?;
        stream.write_u8(0)?; // class

        match &self.response {
            Some(ResponseData::Response { flags, protocol }) => {
                stream.write_u8(Message::Response.to_u8().unwrap())?;
                stream.write_u8(flags.bits())?;
                stream.write_u16::<LittleEndian>(RDP_NEG_DATA_LENGTH)?;
                stream.write_u32::<LittleEndian>(protocol.bits())?;
            }
            Some(ResponseData::Failure { code }) => {
                stream.write_u8(Message::Failure.to_u8().unwrap())?;
                stream.write_u8(0)?; // flags
                stream.write_u16::<LittleEndian>(RDP_NEG_DATA_LENGTH)?;
                stream.write_u32::<LittleEndian>(code.to_u32().unwrap())?;
            }
            None => (),
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        TPDU_REQUEST_LENGTH + RDP_NEG_DATA_LENGTH as usize
    }
}

fn read_nego_data(stream: &[u8]) -> Option<(NegoData, usize)> {
    if let Ok((routing_token, read_len)) = read_string_with_cr_lf(stream, ROUTING_TOKEN_PREFIX) {
        Some((NegoData::RoutingToken(routing_token), read_len))
    } else if let Ok((cookie, read_len)) = read_string_with_cr_lf(stream, COOKIE_PREFIX) {
        Some((NegoData::Cookie(cookie), read_len))
    } else {
        None
    }
}

fn read_string_with_cr_lf(
    mut stream: impl io::BufRead,
    start: &str,
) -> io::Result<(String, usize)> {
    let mut read_start = String::new();
    stream
        .by_ref()
        .take(start.len() as u64)
        .read_to_string(&mut read_start)?;

    if read_start == start {
        let mut value = String::new();
        stream.read_line(&mut value)?;
        match value.pop() {
            Some('\n') => (),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "message uncorrectly terminated",
                ));
            }
        }
        match value.pop() {
            Some('\r') => (), // cr
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "message is not terminated with cr",
                ));
            }
        }
        let value_len = value.len();

        Ok((value, start.len() + value_len + CR_LF_SEQ_LENGTH))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid or unsuppored message",
        ))
    }
}

fn read_and_check_class(
    mut stream: impl io::Read,
    required_class: u8,
) -> Result<(), NegotiationError> {
    let class = stream.read_u8()?;

    if class != required_class {
        return Err(NegotiationError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid tpdu class",
        )));
    }

    Ok(())
}
