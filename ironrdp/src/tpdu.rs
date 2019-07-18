#[cfg(test)]
mod tests;

use std::io;

use byteorder::{BigEndian, LittleEndian, ReadBytesExt, WriteBytesExt};
use bytes::BytesMut;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

pub const TPKT_HEADER_LENGTH: usize = 4;
pub const TPDU_DATA_LENGTH: usize = TPKT_HEADER_LENGTH + TPDU_DATA_HEADER_LENGTH;
pub const TPDU_REQUEST_LENGTH: usize = TPKT_HEADER_LENGTH + TPDU_REQUEST_HEADER_LENGTH;

const TPDU_DATA_HEADER_LENGTH: usize = 3;
const TPDU_REQUEST_HEADER_LENGTH: usize = 7;

/// The PDU type of the X.224 negotiation phase.
#[derive(Copy, Clone, Debug, PartialEq, FromPrimitive, ToPrimitive)]
pub enum X224TPDUType {
    ConnectionRequest = 0xE0,
    ConnectionConfirm = 0xD0,
    DisconnectRequest = 0x80,
    Data = 0xF0,
    Error = 0x70,
}

/// Extracts a [X.224 message type code](enum.X224TPDUType.html)
/// and a buffer ready for parsing from a raw request buffer provided
/// by the argument.
///
/// # Arguments
///
/// * `input` - the raw buffer of the request (e.g. extracted from a stream)
pub fn decode_x224(input: &mut BytesMut) -> io::Result<(X224TPDUType, BytesMut)> {
    let mut stream = input.as_ref();
    let len = read_tpkt_len(&mut stream)? as usize;

    if input.len() < len {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "The buffer length is less then the real length",
        ));
    }

    let (_, code) = parse_tdpu_header(&mut stream)?;

    let mut tpdu = input.split_to(len as usize);
    let header_len = tpdu_header_length(code);
    if header_len <= tpdu.len() {
        tpdu.advance(header_len);

        Ok((code, tpdu))
    } else {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "TPKT len is too small",
        ))
    }
}

/// Encodes and writes the message to an output buffer composed from a
/// [request code](enum.X224TPDUType.html) and a data buffer provided by
/// the arguments.
///
/// # Arguments
///
/// * `code` - the [X.224 request type code](enum.X224TPDUType.html)
/// * `data` - the message data to be encoded
/// * `output` - the output buffer for the encoded data
pub fn encode_x224(code: X224TPDUType, data: BytesMut, output: &mut BytesMut) -> io::Result<()> {
    let tpdu_length = match code {
        X224TPDUType::Data => TPDU_DATA_LENGTH,
        _ => TPDU_REQUEST_LENGTH,
    };

    let length = tpdu_length + data.len();
    let mut output_slice = output.as_mut();
    write_tpkt_header(&mut output_slice, length as u16)?;
    write_tpdu_header(
        &mut output_slice,
        length as u8 - TPKT_HEADER_LENGTH as u8,
        code,
        0,
    )?;

    output.extend_from_slice(&data);

    Ok(())
}

/// Returns TPDU header length using a [X.224 message type code](enum.X224TPDUType.html).
///
/// # Arguments
///
/// * `code` - the [X.224 request type code](enum.X224TPDUType.html)
pub fn tpdu_header_length(code: X224TPDUType) -> usize {
    match code {
        X224TPDUType::Data => TPDU_DATA_LENGTH,
        _ => TPDU_REQUEST_LENGTH,
    }
}

/// Writes the TPKT header to an output source.
///
/// # Arguments
///
/// * `stream` - the output buffer
/// * `length` - the length of the header
pub fn write_tpkt_header(mut stream: impl io::Write, length: u16) -> io::Result<()> {
    let version = 3;

    stream.write_u8(version)?;
    stream.write_u8(0)?; // reserved
    stream.write_u16::<BigEndian>(length)?;

    Ok(())
}

/// Reads a data source containing the TPKT header and returns its length upon success.
///
/// # Arguments
///
/// * `stream` - the data type that contains the header
pub fn read_tpkt_len(mut stream: impl io::Read) -> io::Result<u64> {
    let version = stream.read_u8()?;
    if version != 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a tpkt header",
        ));
    }

    let _reserved = stream.read_u8()?;
    let len = u64::from(stream.read_u16::<BigEndian>()?);

    Ok(len)
}

fn write_tpdu_header(
    mut stream: impl io::Write,
    length: u8,
    code: X224TPDUType,
    src_ref: u16,
) -> io::Result<()> {
    let tpdu_length = match code {
        X224TPDUType::Data => 2,
        _ => length - 1, // tpdu header length field doesn't include the length of the length field
    };

    stream.write_u8(tpdu_length)?;
    stream.write_u8(code.to_u8().unwrap())?;

    if code == X224TPDUType::Data {
        let eot = 0x80;
        stream.write_u8(eot)?;
    } else {
        let dst_ref = 0;
        stream.write_u16::<LittleEndian>(dst_ref)?;
        stream.write_u16::<LittleEndian>(src_ref)?;
        let class = 0;
        stream.write_u8(class)?;
    }

    Ok(())
}

fn parse_tdpu_header(mut stream: impl io::Read) -> io::Result<(u8, X224TPDUType)> {
    let length = stream.read_u8()?;
    let code = X224TPDUType::from_u8(stream.read_u8()?)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid X224 TPDU type"))?;

    if code == X224TPDUType::Data {
        let _eof = stream.read_u8()?;
    } else {
        let _dst_ref = stream.read_u16::<LittleEndian>()?;
        let _src_ref = stream.read_u16::<LittleEndian>()?;
        let _class = stream.read_u8()?;
    }

    Ok((length, code))
}
