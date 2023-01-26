//! PDUs used during the Connection Initiation stage

use std::io;
use std::io::prelude::*;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
use thiserror::Error;

use crate::x224::{TpktHeader, X224TPDUType, TPDU_REQUEST_LENGTH, TPKT_HEADER_LENGTH};
use crate::PduParsing;

const COOKIE_PREFIX: &str = "Cookie: mstshash=";
const ROUTING_TOKEN_PREFIX: &str = "Cookie: msts=";

const RDP_NEG_DATA_LENGTH: u16 = 8;
const CR_LF_SEQ_LENGTH: usize = 2;

bitflags! {
    /// A 32-bit, unsigned integer that contains flags indicating the supported
    /// security protocols.
    /// The client and server agree on it during the Connection Initiation phase.
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, FromPrimitive, ToPrimitive)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NegoData {
    RoutingToken(String),
    Cookie(String),
}

///  The type of the error that may result from a negotiation process.
#[derive(Debug, Error)]
pub enum NegotiationError {
    /// Corresponds for an I/O error that may occur during a negotiation process
    /// (invalid response code, invalid security protocol code, etc.)
    #[error("IO error")]
    IOError(#[from] io::Error),
    /// May indicate about a negotiation error recieved from a server.
    #[error("Received negotiation error from server, code={0:?}")]
    ResponseFailure(FailureCode),
    #[error("Invalid tpkt header version")]
    TpktVersionError,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub nego_data: Option<NegoData>,
    pub flags: RequestFlags,
    pub protocol: SecurityProtocol,
    pub src_ref: u16,
}

impl PduParsing for Request {
    type Error = NegotiationError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let tpkt = TpktHeader::from_buffer(&mut stream)?;

        crate::x224::read_and_check_tpdu_header(&mut stream, X224TPDUType::ConnectionRequest)?;

        let _dst_ref = stream.read_u16::<LittleEndian>()?;
        let src_ref = stream.read_u16::<LittleEndian>()?;

        read_and_check_class(&mut stream, 0)?;

        let mut buffer = vec![0u8; tpkt.length - TPDU_REQUEST_LENGTH];

        stream.read_exact(buffer.as_mut_slice())?;
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
            Some(NegoData::RoutingToken(s)) => writeln!(&mut stream, "{}{}\r", ROUTING_TOKEN_PREFIX, s)?,
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
                Some(NegoData::RoutingToken(s)) => s.len() + ROUTING_TOKEN_PREFIX.len() + CR_LF_SEQ_LENGTH,
                None => 0,
            }
            + if self.protocol.bits() > SecurityProtocol::RDP.bits() {
                usize::from(RDP_NEG_DATA_LENGTH)
            } else {
                0
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseData {
    Confirm {
        flags: ResponseFlags,
        protocol: SecurityProtocol,
    },
    Failure {
        code: FailureCode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
                let protocol = SecurityProtocol::from_bits_truncate(stream.read_u32::<LittleEndian>()?);

                Ok(Self {
                    response: Some(ResponseData::Confirm { flags, protocol }),
                    dst_ref,
                    src_ref,
                })
            }
            Message::Failure => {
                let error = FailureCode::from_u32(stream.read_u32::<LittleEndian>()?).ok_or_else(|| {
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
            Some(ResponseData::Confirm { flags, protocol }) => {
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

fn read_string_with_cr_lf(mut stream: impl io::BufRead, start: &str) -> io::Result<(String, usize)> {
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

fn read_and_check_class(mut stream: impl io::Read, required_class: u8) -> Result<(), NegotiationError> {
    let class = stream.read_u8()?;

    if class != required_class {
        return Err(NegotiationError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid tpdu class",
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rdp_negotiation_data_is_written_to_request_if_nla_security() {
        let mut buffer = Vec::new();
        let expected = [0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00];

        let request = Request {
            nego_data: Some(NegoData::Cookie("a".to_string())),
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
            src_ref: 0,
        };

        request.to_buffer(&mut buffer).unwrap();

        assert_eq!(buffer[buffer.len() - usize::from(RDP_NEG_DATA_LENGTH)..], expected);
    }

    #[test]
    fn rdp_negotiation_data_is_not_written_if_rdp_security() {
        #[rustfmt::skip]
        let expected = [
            // tpkt header
            0x3u8, // version
            0x0, // reserved
            0x00, 0x22, // lenght in BE

            // tpdu
            0x1d, // length
            0xe0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
            0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A,
        ];
        let mut buff = Vec::new();

        let request = Request {
            nego_data: Some(NegoData::Cookie("User".to_string())),
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::RDP,
            src_ref: 0,
        };

        request.to_buffer(&mut buff).unwrap();

        assert_eq!(expected.as_ref(), buff.as_slice());
    }

    #[test]
    fn negotiation_request_is_written_correclty() {
        #[rustfmt::skip]
        let expected = [
            // tpkt header
            0x3u8, // version
            0x0, // reserved
            0x00, 0x2a, // lenght in BE

            // tpdu
            0x25, // length
            0xe0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
            0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A, 0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00,
            0x00,
        ];
        let mut buff = Vec::new();

        let request = Request {
            nego_data: Some(NegoData::Cookie("User".to_string())),
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
            src_ref: 0,
        };

        request.to_buffer(&mut buff).unwrap();

        assert_eq!(expected.as_ref(), buff.as_slice());
    }

    #[test]
    fn negotiation_response_is_processed_correctly() {
        let expected_flags = ResponseFlags::all();

        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x13, // lenght in BE

            // tpdu
            0x6, // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x02, // negotiation message
            expected_flags.bits(),
            0x08, 0x00, // length
            0x02, 0x00, 0x00, 0x00, // selected protocol
        ];

        let response_data = Some(ResponseData::Confirm {
            flags: expected_flags,
            protocol: SecurityProtocol::HYBRID,
        });

        let response = Response {
            response: response_data,
            dst_ref: 0,
            src_ref: 0,
        };

        assert_eq!(response, Response::from_buffer(buffer.as_ref()).unwrap());
    }

    #[test]
    fn wrong_message_code_in_negotiation_response_results_in_error() {
        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x13, // lenght in BE

            // tpdu
            0x6,  // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0xAF, // negotiation message
            0x1F, // flags
            0x08, 0x00, // length
            0x02, 0x00, 0x00, 0x00, // selected protocol
        ];

        match Response::from_buffer(buffer.as_ref()) {
            Err(NegotiationError::IOError(ref e)) if e.kind() == io::ErrorKind::InvalidData => (),
            Err(e) => panic!("wrong error type: {}", e),
            _ => panic!("error expected"),
        }
    }

    #[test]
    fn negotiation_failure_in_response_results_in_error() {
        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x13, // lenght in BE

            // tpdu
            0x6,  // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x03, // negotiation message
            0x1F, // flags
            0x08, 0x00, // length
            0x06, 0x00, 0x00, 0x00, // failure code
        ];

        match Response::from_buffer(buffer.as_ref()) {
            Err(NegotiationError::ResponseFailure(e)) if e == FailureCode::SSLWithUserAuthRequiredByServer => {}
            Err(e) => panic!("wrong error type: {}", e),
            _ => panic!("error expected"),
        }
    }

    #[test]
    fn cookie_in_request_is_parsed_correctly() {
        let request = [
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
            0x73, 0x65, 0x72, 0x0D, 0x0A, 0xFF, 0xFF,
        ];

        let (nego_data, _read_len) = read_nego_data(request.as_ref()).unwrap();

        match nego_data {
            NegoData::Cookie(cookie) => assert_eq!(cookie, "User"),
            _ => panic!("Cookie expected"),
        };
    }

    #[test]
    fn routing_token_in_request_is_parsed_correctly() {
        let request = [
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x3D, 0x33, 0x36, 0x34, 0x30, 0x32,
            0x30, 0x35, 0x32, 0x32, 0x38, 0x2E, 0x31, 0x35, 0x36, 0x32, 0x39, 0x2E, 0x30, 0x30, 0x30, 0x30, 0x0D, 0x0A,
            0xFF, 0xFF,
        ];

        let (nego_data, _read_len) = read_nego_data(request.as_ref()).unwrap();

        match nego_data {
            NegoData::RoutingToken(routing_token) => assert_eq!(routing_token, "3640205228.15629.0000"),
            _ => panic!("Routing token expected"),
        };
    }

    #[test]
    fn read_string_with_cr_lf_on_non_value_results_in_error() {
        let request = [
            0x6e, 0x6f, 0x74, 0x20, 0x61, 0x20, 0x63, 0x6f, 0x6f, 0x6b, 0x69, 0x65, 0x0F, 0x42, 0x73, 0x65, 0x72, 0x0D,
            0x0A, 0xFF, 0xFF,
        ];

        match read_string_with_cr_lf(&mut request.as_ref(), COOKIE_PREFIX) {
            Err(ref e) if e.kind() == io::ErrorKind::InvalidData => (),
            Err(e) => panic!("wrong error type: {}", e),
            _ => panic!("error expected"),
        }
    }

    #[test]
    fn read_string_with_cr_lf_on_unterminated_message_results_in_error() {
        let request = [
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
            0x73, 0x65, 0x72,
        ];

        match read_string_with_cr_lf(&mut request.as_ref(), COOKIE_PREFIX) {
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => (),
            Err(e) => panic!("wrong error type: {}", e),
            _ => panic!("error expected"),
        }
    }

    #[test]
    fn read_string_with_cr_lf_on_unterminated_with_cr_message() {
        let request = [
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x0a,
        ];

        match read_string_with_cr_lf(&mut request.as_ref(), COOKIE_PREFIX) {
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => (),
            Err(e) => panic!("wrong error type: {}", e),
            _ => panic!("error expected"),
        }
    }

    #[test]
    fn negotiation_request_with_negotiation_data_is_parsed_correctly() {
        let expected_flags =
            RequestFlags::RESTRICTED_ADMIN_MODE_REQUIRED | RequestFlags::REDIRECTED_AUTHENTICATION_MODE_REQUIRED;

        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x2a, // lenght in BE

            // tpdu
            0x6, // length
            0xe0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D, 0x55,
            0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
            0x01, // request code
            expected_flags.bits(),
            0x08, 0x00, 0x03, 0x00, 0x00, 0x00, // request message
        ];

        let request = Request {
            nego_data: Some(NegoData::Cookie("User".to_string())),
            flags: expected_flags,
            protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
            src_ref: 0,
        };

        assert_eq!(request, Request::from_buffer(buffer.as_ref()).unwrap());
    }

    #[test]
    fn negotiation_request_without_variable_fields_is_parsed_correctly() {
        let expected_flags =
            RequestFlags::RESTRICTED_ADMIN_MODE_REQUIRED | RequestFlags::REDIRECTED_AUTHENTICATION_MODE_REQUIRED;

        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x13, // lenght in BE
            // tpdu
            0x6, // length
            0xe0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x01, // request code
            expected_flags.bits(),
            0x08, 0x00, 0x03, 0x00, 0x00, 0x00, // request message
        ];

        let request = Request {
            nego_data: None,
            flags: expected_flags,
            protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
            src_ref: 0,
        };

        assert_eq!(request, Request::from_buffer(buffer.as_ref()).unwrap());
    }

    #[test]
    fn negotiation_request_without_negotiation_data_is_parsed_correctly() {
        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x22, // lenght in BE
            // tpdu
            0x6,  // length
            0xe0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
            0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
        ];

        let request = Request {
            nego_data: Some(NegoData::Cookie("User".to_string())),
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::RDP,
            src_ref: 0,
        };

        assert_eq!(request, Request::from_buffer(buffer.as_ref()).unwrap());
    }

    #[test]
    fn negotiation_request_with_invalid_negotiation_code_results_in_error() {
        #[rustfmt::skip]
        let request = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x2a, // lenght in BE
            // tpdu
            0x6,  // length
            0xe0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
            0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A, // cookie
            0x03, // request code
            0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00, // request message
        ];

        match Request::from_buffer(request.as_ref()) {
            Err(NegotiationError::IOError(ref e)) if e.kind() == io::ErrorKind::InvalidData => (),
            Err(e) => panic!("wrong error type: {}", e),
            _ => panic!("error expected"),
        }
    }

    #[test]
    fn negotiation_response_is_written_correctly() {
        let flags = ResponseFlags::all();

        #[rustfmt::skip]
        let expected = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x13, // lenght in BE

            // tpdu
            0x0e, // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x02, // negotiation message
            flags.bits(),
            0x08, 0x00, // length
            0x02, 0x00, 0x00, 0x00, // selected protocol
        ];

        let mut buffer = Vec::new();

        let response_data = Some(ResponseData::Confirm {
            flags,
            protocol: SecurityProtocol::HYBRID,
        });

        let response = Response {
            response: response_data,
            dst_ref: 0,
            src_ref: 0,
        };

        response.to_buffer(&mut buffer).unwrap();

        assert_eq!(buffer, expected);
    }

    #[test]
    fn negotiation_error_is_written_correclty() {
        #[rustfmt::skip]
        let expected = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x13, // lenght in BE

            // tpdu
            0x0e, // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x03, // negotiation message
            0x00,
            0x08, 0x00, // length
            0x02, 0x00, 0x00, 0x00, // error code
        ];

        let mut buffer = Vec::new();

        let failure_data = Some(ResponseData::Failure {
            code: FailureCode::SSLNotAllowedByServer,
        });

        let response = Response {
            response: failure_data,
            dst_ref: 0,
            src_ref: 0,
        };

        response.to_buffer(&mut buffer).unwrap();

        assert_eq!(buffer, expected);
    }

    #[test]
    fn buffer_length_is_correct_for_negatiation_request() {
        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x2a, // lenght in BE

            // tpdu
            0x6,  // length
            0xe0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73,
            0x68, 0x3D, 0x55, 0x73, 0x65, 0x72, 0x0D, 0x0A, 0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00,
            0x00,
        ];

        let request = Request {
            nego_data: Some(NegoData::Cookie("User".to_string())),
            flags: RequestFlags::empty(),
            protocol: SecurityProtocol::HYBRID | SecurityProtocol::SSL,
            src_ref: 0,
        };

        assert_eq!(request.buffer_length(), buffer.len());
    }

    #[test]
    fn buffer_length_is_correct_for_negotiation_response() {
        let flags = ResponseFlags::all();
        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00,
            0x13, // lenght in BE

            // tpdu
            0x6,  // length
            0xd0, // code
            0x00,
            0x00, // dst_ref
            0x00,
            0x00, // src_ref
            0x00, // class

            0x02, // negotiation message
            flags.bits(),
            0x08,
            0x00, // length
            0x02,
            0x00,
            0x00,
            0x00, // selected protocol
        ];

        let response_data = Some(ResponseData::Confirm {
            flags,
            protocol: SecurityProtocol::HYBRID,
        });

        let response = Response {
            response: response_data,
            dst_ref: 0,
            src_ref: 0,
        };

        assert_eq!(response.buffer_length(), buffer.len());
    }

    #[test]
    fn from_buffer_correctly_parses_negotiation_failure() {
        #[rustfmt::skip]
        let expected = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x13, // lenght in BE

            // tpdu
            0x6, // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x03, // negotiation message
            0x00,
            0x08, 0x00, // length
            0x02, 0x00, 0x00, 0x00, // error code
        ];

        match Response::from_buffer(expected.as_ref()) {
            Err(NegotiationError::ResponseFailure(_)) => (),
            Err(e) => panic!("invalid error type: {}", e),
            Ok(_) => panic!("error expected"),
        }
    }

    #[test]
    fn buffer_length_is_correct_for_negotiation_failure() {
        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x13, // lenght in BE

            // tpdu
            0x6,  // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class

            0x03, // negotiation message
            0x00, 0x08, 0x00, // length
            0x02, 0x00, 0x00, 0x00, // error code
        ];

        let failure_data = Some(ResponseData::Failure {
            code: FailureCode::SSLNotAllowedByServer,
        });

        let failure = Response {
            response: failure_data,
            dst_ref: 0,
            src_ref: 0,
        };

        assert_eq!(buffer.len(), failure.buffer_length());
    }

    #[test]
    fn read_and_check_tpdu_header_reads_invalid_data_correctly() {
        let buffer = [
            0x6,  // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class
        ];

        assert!(crate::x224::read_and_check_tpdu_header(buffer.as_ref(), X224TPDUType::Data).is_err());
    }

    #[test]
    fn read_and_check_tpdu_header_reads_correct_data_correctly() {
        let buffer = [
            0x6,  // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x00, // class
        ];

        crate::x224::read_and_check_tpdu_header(buffer.as_ref(), X224TPDUType::ConnectionConfirm).unwrap();
    }

    #[test]
    fn invalid_class_is_handeled_correctly() {
        #[rustfmt::skip]
        let buffer = [
            // tpkt header
            0x3, // version
            0x0, // reserved
            0x00, 0x13, // lenght in BE

            // tpdu
            0x6, // length
            0xd0, // code
            0x00, 0x00, // dst_ref
            0x00, 0x00, // src_ref
            0x01, // class

            0x03, // negotiation message
            0x00,
            0x08, 0x00, // length
            0x02, 0x00, 0x00, 0x00, // error code
        ];

        assert!(Response::from_buffer(buffer.as_ref()).is_err());
    }

    #[test]
    fn parse_negotiation_request_correctly_handles_invalid_slice_length() {
        let request = [
            0x43, 0x6F, 0x6F, 0x6B, 0x69, 0x65, 0x3A, 0x20, 0x6D, 0x73, 0x74, 0x73, 0x68, 0x61, 0x73, 0x68, 0x3D,
            0x0a, // failing cookie
        ];

        match Request::from_buffer(request.as_ref()) {
            Err(NegotiationError::TpktVersionError) => (),
            Err(e) => panic!("wrong error type: {}", e),
            _ => panic!("error expected"),
        }
    }
}
