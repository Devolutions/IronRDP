#[cfg(test)]
mod test;

use std::io;

use byteorder::ReadBytesExt;
use failure::Fail;

use crate::{impl_from_error, per};

/// Implements the Fast-Path RDP message header PDU.
#[derive(Debug)]
pub struct FastPath {
    pub encryption_flags: u8,
    pub number_events: u8,
    pub length: u16,
}

/// Parses the data received as an argument and returns a
/// [`Fastpath`](struct.Fastpath.html) structure upon success.
///
/// # Arguments
///
/// * `stream` - the type to read data from
pub fn parse_fast_path_header(mut stream: impl io::Read) -> Result<(FastPath, u16), FastPathError> {
    let header = stream.read_u8()?;

    let (length, sizeof_length) = per::read_length(&mut stream)?;
    if length < sizeof_length as u16 + 1 {
        return Err(FastPathError::NullLength {
            bytes_read: sizeof_length as usize + 1,
        });
    }

    let pdu_length = length - sizeof_length as u16 - 1;

    Ok((
        FastPath {
            encryption_flags: (header & 0xC0) >> 6,
            number_events: (header & 0x3C) >> 2,
            length: pdu_length,
        },
        length,
    ))
}

/// The type of a Fast-Path parsing error. Includes *length error* and *I/O error*.
#[derive(Debug, Fail)]
pub enum FastPathError {
    /// May be used in I/O related errors such as receiving empty Fast-Path packages.
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    /// Used in the length-related error during Fast-Path parsing.
    #[fail(display = "Received invalid Fast-Path package with 0 length")]
    NullLength { bytes_read: usize },
}

impl_from_error!(io::Error, FastPathError, FastPathError::IOError);
