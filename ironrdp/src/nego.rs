#[cfg(test)]
mod tests;

use std::{
    error::Error,
    fmt,
    io::{self, prelude::*},
};

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::tpdu::X224TPDUType;

pub const NEGOTIATION_REQUEST_LEN: usize = 27;
pub const NEGOTIATION_RESPONSE_LEN: usize = 8;

const COOKIE_PREFIX: &str = "Cookie: mstshash=";
const ROUTING_TOKEN_PREFIX: &str = "Cookie: msts=";

const RDP_NEG_DATA_LENGTH: u16 = 8;

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
    pub struct NegotiationRequestFlags: u8 {
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
    pub struct NegotiationResponseFlags: u8 {
        const EXTENDED_CLIENT_DATA_SUPPORTED = 0x01;
        const DYNVC_GFX_PROTOCOL_SUPPORTED = 0x02;
        const RDP_NEG_RSP_RESERVED = 0x04;
        const RESTRICTED_ADMIN_MODE_SUPPORTED = 0x08;
        const REDIRECTED_AUTHENTICATION_MODE_SUPPORTED = 0x10;
    }
}

/// The type of the negotiation error. Contained in
/// [`NegotiationError`](enum.NegotiationError.html).
#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum NegotiationFailureCodes {
    SSLRequiredByServer = 1,
    SSLNotAllowedByServer = 2,
    SSLCertNotOnServer = 3,
    InconsistentFlags = 4,
    HybridRequiredByServer = 5,
    /// Used when the failure caused by [`NegotiationFailure`](struct.NegotiationFailure.html).
    SSLWithUserAuthRequiredByServer = 6,
}

/// The kind of the negotiation request message, including the message as a
/// [`String`](https://doc.rust-lang.org/std/string/struct.String.html).
///
/// # MSDN
///
/// * [Client X.224 Connection Request PDU](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/18a27ef9-6f9a-4501-b000-94b1fe3c2c10)
#[derive(PartialEq, Debug)]
pub enum NegoData {
    RoutingToken(String),
    Cookie(String),
}

///  The type of the error that may result from a negotiation process.
#[derive(Debug)]
pub enum NegotiationError {
    /// Corresponds for an I/O error that may occur during a negotiation process
    /// (invalid response code, invalid security protocol code, etc.)
    IOError(io::Error),
    /// May indicate about a negotiation error recieved from a server.
    NegotiationFailure(NegotiationFailureCodes),
}

impl fmt::Display for NegotiationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NegotiationError::IOError(e) => e.fmt(f),
            NegotiationError::NegotiationFailure(code) => {
                write!(f, "Received negotiation error from server, code={:?}", code)
            }
        }
    }
}

impl Error for NegotiationError {}

impl From<io::Error> for NegotiationError {
    fn from(e: io::Error) -> Self {
        NegotiationError::IOError(e)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
enum NegotiationMessage {
    Request = 1,
    Response = 2,
    Failure = 3,
}

/// Writes the negotiation request to the output buffer. The request is composed
/// of a cookie string, a [security protocol](struct.SecurityProtocol.html), and
/// [negotiation request flags](struct.NegotiationRequestFlags.html).
///
/// # Arguments
///
/// * `buffer` - the output buffer
/// * `cookie` - the cookie string slice
/// * `protocol` - the [security protocol](struct.SecurityProtocol.html) of the message
/// * `flags` - the [negotiation request flags](struct.NegotiationRequestFlags.html)
///
/// # MSDN
///
/// * [Client X.224 Connection Request PDU](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/18a27ef9-6f9a-4501-b000-94b1fe3c2c10)
/// * [RDP Negotiation Request (RDP_NEG_REQ)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/902b090b-9cb3-4efc-92bf-ee13373371e3)
pub fn write_negotiation_request(
    mut buffer: impl io::Write,
    cookie: &str,
    protocol: SecurityProtocol,
    flags: NegotiationRequestFlags,
) -> io::Result<()> {
    writeln!(buffer, "{}{}\r", COOKIE_PREFIX, cookie)?;

    if protocol.bits() > SecurityProtocol::RDP.bits() {
        write_negotiation_data(
            buffer,
            NegotiationMessage::Request,
            flags.bits(),
            protocol.bits(),
        )?;
    }

    Ok(())
}

/// Parses the negotiation request represented by the arguments and returns a tuple with
/// [negotiation data](enum.NegoData.html) (optional), a [security protocol](struct.SecurityProtocol.html),
/// and [negotiation request flags](struct.NegotiationRequestFlags.html).
///
/// # Arguments
///
/// * `code` - the [type](enum.X224TPDUType.html) of the X.224 request
/// * `slice` - the input buffer of the request
///
/// # MSDN
///
/// * [Client X.224 Connection Request PDU](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/18a27ef9-6f9a-4501-b000-94b1fe3c2c10)
/// * [RDP Negotiation Request (RDP_NEG_REQ)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/902b090b-9cb3-4efc-92bf-ee13373371e3)
pub fn parse_negotiation_request(
    code: X224TPDUType,
    mut slice: &[u8],
) -> io::Result<(Option<NegoData>, SecurityProtocol, NegotiationRequestFlags)> {
    if code != X224TPDUType::ConnectionRequest {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected X224 connection request",
        ));
    }

    let nego_data = if let Some((nego_data, read_len)) = read_nego_data(slice) {
        slice.consume(read_len);

        Some(nego_data)
    } else {
        None
    };

    if slice.len() >= 8 {
        let neg_req = NegotiationMessage::from_u8(slice.read_u8()?).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid negotiation request code",
            )
        })?;
        if neg_req != NegotiationMessage::Request {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid negotiation request code",
            ));
        }

        let flags = NegotiationRequestFlags::from_bits(slice.read_u8()?).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid negotiation request flags",
            )
        })?;
        let _length = slice.read_u16::<LittleEndian>()?;
        let protocol =
            SecurityProtocol::from_bits(slice.read_u32::<LittleEndian>()?).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid security protocol code")
            })?;

        Ok((nego_data, protocol, flags))
    } else {
        Ok((
            nego_data,
            SecurityProtocol::RDP,
            NegotiationRequestFlags::default(),
        ))
    }
}

/// Writes the negotiation response to an output buffer. The response is composed
/// of a [security protocol](struct.SecurityProtocol.html) and
/// [negotiation response flags](struct.NegotiationRequestFlags.html).
///
/// # Arguments
///
/// * `buffer` - the output buffer
/// * `flags` - the [negotiation response flags](struct.NegotiationResponseFlags.html)
/// * `protocol` - the [security protocol](struct.SecurityProtocol.html) of the message
///
/// # MSDN
///
/// * [Server X.224 Connection Confirm PDU](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/13757f8f-66db-4273-9d2c-385c33b1e483)
/// * [RDP Negotiation Response (RDP_NEG_RSP)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/b2975bdc-6d56-49ee-9c57-f2ff3a0b6817)
pub fn write_negotiation_response(
    buffer: impl io::Write,
    flags: NegotiationResponseFlags,
    protocol: SecurityProtocol,
) -> io::Result<()> {
    write_negotiation_data(
        buffer,
        NegotiationMessage::Response,
        flags.bits(),
        protocol.bits(),
    )
}

/// Writes the negotiation response error to an output buffer.
///
/// # Arguments
///
/// * `buffer` - the output buffer
/// * `error` - the [failure code](enum.NegotiationFailureCodes.html)
pub fn write_negotiation_response_error(
    buffer: impl io::Write,
    error: NegotiationFailureCodes,
) -> io::Result<()> {
    write_negotiation_data(
        buffer,
        NegotiationMessage::Failure,
        0,
        error.to_u32().unwrap() & !0x8000_0000,
    )
}

/// Parses the negotiation response represented by the arguments and
/// returns a tuple with a [security protocol](struct.SecurityProtocol.html)
/// and [negotiation response flags](struct.NegotiationResponseFlags.html)
/// upon success.
///
/// # Arguments
///
/// * `code` - the [type](enum.X224TPDUType.html) of the X.224 message
/// * `stream` - the data type that contains the response
///
/// # MSDN
///
/// * [RDP Negotiation Response (RDP_NEG_RSP)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/b2975bdc-6d56-49ee-9c57-f2ff3a0b6817)
pub fn parse_negotiation_response(
    code: X224TPDUType,
    mut stream: impl io::Read,
) -> Result<(SecurityProtocol, NegotiationResponseFlags), NegotiationError> {
    if code != X224TPDUType::ConnectionConfirm {
        return Err(NegotiationError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected X224 connection confirm",
        )));
    }

    let neg_resp = NegotiationMessage::from_u8(stream.read_u8()?).ok_or_else(|| {
        NegotiationError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid negotiation response code",
        ))
    })?;
    let flags = NegotiationResponseFlags::from_bits(stream.read_u8()?).ok_or_else(|| {
        NegotiationError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid negotiation response flags",
        ))
    })?;
    let _length = stream.read_u16::<LittleEndian>()?;

    if neg_resp == NegotiationMessage::Response {
        let selected_protocol = SecurityProtocol::from_bits(stream.read_u32::<LittleEndian>()?)
            .ok_or_else(|| {
                NegotiationError::IOError(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid security protocol code",
                ))
            })?;
        Ok((selected_protocol, flags))
    } else if neg_resp == NegotiationMessage::Failure {
        let error = NegotiationFailureCodes::from_u32(stream.read_u32::<LittleEndian>()?)
            .ok_or_else(|| {
                NegotiationError::IOError(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid security protocol code",
                ))
            })?;
        Err(NegotiationError::NegotiationFailure(error))
    } else {
        Err(NegotiationError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid negotiation response code",
        )))
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
        value.pop(); // cr
        let value_len = value.len();

        Ok((value, start.len() + value_len + 2))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid or unsuppored message",
        ))
    }
}

fn write_negotiation_data(
    mut cursor: impl io::Write,
    message: NegotiationMessage,
    flags: u8,
    data: u32,
) -> io::Result<()> {
    cursor.write_u8(message.to_u8().unwrap())?;
    cursor.write_u8(flags)?;
    cursor.write_u16::<LittleEndian>(RDP_NEG_DATA_LENGTH)?;
    cursor.write_u32::<LittleEndian>(data)?;

    Ok(())
}
