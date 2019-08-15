#[cfg(test)]
mod tests;

use std::io;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{nego::NegotiationError, PduParsing};

pub const TPKT_HEADER_LENGTH: usize = 4;
pub const TPDU_DATA_HEADER_LENGTH: usize = 3;

pub const TPDU_REQUEST_LENGTH: usize = TPKT_HEADER_LENGTH + TPDU_REQUEST_HEADER_LENGTH;
pub const TPDU_REQUEST_HEADER_LENGTH: usize = 7;
pub const TPDU_ERROR_HEADER_LENGTH: usize = 5;

const TPKT_VERSION: u8 = 3;

const EOF: u8 = 0x80;

/// The PDU type of the X.224 negotiation phase.
#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum X224TPDUType {
    ConnectionRequest = 0xE0,
    ConnectionConfirm = 0xD0,
    DisconnectRequest = 0x80,
    Data = 0xF0,
    Error = 0x70,
}

#[derive(PartialEq, Debug)]
pub struct TpktHeader {
    pub length: usize,
}

impl TpktHeader {
    pub fn new(length: usize) -> Self {
        Self { length }
    }
}

impl PduParsing for TpktHeader {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let version = stream.read_u8()?;
        if version != TPKT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid tpkt header version",
            ));
        }

        let _reserved = stream.read_u8()?;
        let length = usize::from(stream.read_u16::<BigEndian>()?);

        Ok(Self { length })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u8(TPKT_VERSION)?;
        stream.write_u8(0)?; // reserved
        stream.write_u16::<BigEndian>(self.length as u16)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        TPKT_HEADER_LENGTH
    }
}

#[derive(Debug, PartialEq)]
pub struct Data {
    pub data_length: usize,
}

impl Data {
    pub fn new(data_length: usize) -> Self {
        Self { data_length }
    }
}

impl PduParsing for Data {
    type Error = NegotiationError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let tpkt = TpktHeader::from_buffer(&mut stream)?;

        read_and_check_tpdu_header(&mut stream, X224TPDUType::Data)?;

        let _eof = stream.read_u8()?;

        let data_length = tpkt.length - tpkt.buffer_length() - TPDU_DATA_HEADER_LENGTH;

        Ok(Self { data_length })
    }

    fn to_buffer(&self, mut stream: impl std::io::Write) -> Result<(), Self::Error> {
        TpktHeader::new(self.buffer_length()).to_buffer(&mut stream)?;

        stream.write_u8(TPDU_DATA_HEADER_LENGTH as u8 - 1)?;
        stream.write_u8(X224TPDUType::Data.to_u8().unwrap())?;
        stream.write_u8(EOF)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        TPKT_HEADER_LENGTH + TPDU_DATA_HEADER_LENGTH + self.data_length
    }
}

pub fn read_and_check_tpdu_header(
    mut stream: impl io::Read,
    required_code: X224TPDUType,
) -> Result<(), NegotiationError> {
    let _tpdu_length = usize::from(stream.read_u8()?);

    let code = X224TPDUType::from_u8(stream.read_u8()?).ok_or_else(|| {
        NegotiationError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid tpdu code",
        ))
    })?;

    if code != required_code {
        return Err(NegotiationError::IOError(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected tpdu code",
        )));
    }

    Ok(())
}
