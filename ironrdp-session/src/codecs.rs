use std::io;
use std::pin::Pin;

use bit_field::BitField;
use byteorder::{BigEndian, ReadBytesExt};
use bytes::{BufMut, BytesMut};
use futures_util::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use ironrdp::Action;
use num_traits::FromPrimitive;

use crate::transport::{Decoder as TransportDecoder, Encoder as TransportEncoder};

pub type ErasedWriter = Pin<Box<dyn AsyncWrite + Send>>;

pub type ErasedReader = Pin<Box<dyn AsyncRead + Send>>;

pub struct FramedReader<R = ErasedReader> {
    reader: R,
    buf: BytesMut,
}

impl<R> FramedReader<R>
where
    R: AsyncRead,
{
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            buf: BytesMut::new(),
        }
    }

    pub fn into_erased(self) -> FramedReader<ErasedReader>
    where
        R: Send + 'static,
    {
        FramedReader {
            reader: Box::pin(self.reader),
            buf: self.buf,
        }
    }

    pub fn into_inner(self) -> (R, BytesMut) {
        (self.reader, self.buf)
    }

    pub async fn read_frame(&mut self) -> Result<Option<BytesMut>, ironrdp::RdpError>
    where
        R: Unpin,
    {
        loop {
            // Try decoding and see if a frame has been received already
            if let Some(frame) = decode_frame(&mut self.buf)? {
                return Ok(Some(frame));
            }

            // NOTE: tokio ecosystem has a nice API for this with `AsyncReadExt::read_buf`
            let mut read_bytes = [0u8; 1024];
            let len = self.reader.read(&mut read_bytes[..]).await?;
            self.buf.extend_from_slice(&read_bytes[..len]);

            // Handle EOF
            if len == 0 {
                let frame = decode_frame_eof(&mut self.buf)?;
                return Ok(frame);
            }
        }
    }

    pub async fn decode_next_frame<D>(&mut self, decoder: &mut D) -> Result<D::Item, crate::RdpError>
    where
        D: TransportDecoder,
        D::Error: Into<crate::RdpError>,
        R: Unpin,
    {
        let frame = self
            .read_frame()
            .await?
            .ok_or(crate::RdpError::UnexpectedStreamTermination)?;

        let item = decoder.decode(&frame[..]).map_err(Into::into)?;

        Ok(item)
    }
}

pub async fn encode_next_frame<W, E>(writer: &mut W, encoder: &mut E, item: E::Item) -> Result<(), crate::RdpError>
where
    W: AsyncWrite + Unpin,
    E: TransportEncoder,
    E::Error: Into<crate::RdpError>,
{
    let mut buf = BytesMut::new();
    let buf_writer = (&mut buf).writer();
    encoder.encode(item, buf_writer).map_err(Into::into)?;
    writer.write_all(&buf).await?;
    Ok(())
}

/// Function to call when there are no more bytes available to be read from the underlying I/O.
fn decode_frame_eof(buf: &mut BytesMut) -> Result<Option<BytesMut>, ironrdp::RdpError> {
    match decode_frame(buf)? {
        Some(frame) => Ok(Some(frame)),
        None => {
            if buf.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(io::ErrorKind::Other, "expected more bytes (remaining on stream?)").into())
            }
        }
    }
}

/// Attempts to decode a frame from the provided buffer of bytes.
fn decode_frame(buf: &mut BytesMut) -> Result<Option<BytesMut>, ironrdp::RdpError> {
    let mut stream = buf.as_ref();
    if stream.is_empty() {
        return Ok(None);
    }
    let header = stream.read_u8()?;
    let action = header.get_bits(0..2);
    let action = Action::from_u8(action).ok_or(ironrdp::RdpError::InvalidActionCode(action))?;

    let length = match action {
        Action::X224 if stream.len() >= 3 => {
            let _reserved = stream.read_u8()?;

            stream.read_u16::<BigEndian>()?
        }
        Action::FastPath if !stream.is_empty() => {
            let a = stream.read_u8()?;
            if a & 0x80 != 0 {
                if stream.is_empty() {
                    return Ok(None);
                }
                let b = stream.read_u8()?;
                ((u16::from(a) & !0x80) << 8) + u16::from(b)
            } else {
                u16::from(a)
            }
        }
        _ => {
            return Ok(None);
        }
    };

    if buf.len() >= length as usize {
        Ok(Some(buf.split_to(length as usize)))
    } else {
        Ok(None)
    }
}
