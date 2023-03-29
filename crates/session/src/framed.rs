use std::io;
use std::pin::Pin;

use bit_field::BitField;
use byteorder::{BigEndian, ReadBytesExt};
use bytes::{BufMut, BytesMut};
use futures_util::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use ironrdp_pdu::Action;
use num_traits::FromPrimitive;

use crate::frame::Frame;

#[cfg(not(feature = "dgw_ext"))]
pub type ErasedWriter = Pin<Box<dyn AsyncWrite + Send>>;
#[cfg(feature = "dgw_ext")]
pub type ErasedWriter = Pin<Box<dyn AsyncWrite>>;

#[cfg(not(feature = "dgw_ext"))]
pub type ErasedReader = Pin<Box<dyn AsyncRead + Send>>;
#[cfg(feature = "dgw_ext")]
pub type ErasedReader = Pin<Box<dyn AsyncRead>>;

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

    #[cfg(not(feature = "dgw_ext"))]
    pub fn into_erased(self) -> FramedReader<ErasedReader>
    where
        R: Send + 'static,
    {
        FramedReader {
            reader: Box::pin(self.reader),
            buf: self.buf,
        }
    }

    #[cfg(feature = "dgw_ext")]
    pub fn into_erased(self) -> FramedReader<ErasedReader>
    where
        R: 'static,
    {
        FramedReader {
            reader: Box::pin(self.reader),
            buf: self.buf,
        }
    }

    pub fn into_inner(self) -> (R, BytesMut) {
        (self.reader, self.buf)
    }

    pub fn into_inner_no_leftover(self) -> R {
        let (reader, leftover) = self.into_inner();
        debug_assert_eq!(leftover.len(), 0, "unexpected leftover");
        reader
    }

    pub fn get_inner(&self) -> (&R, &BytesMut) {
        (&self.reader, &self.buf)
    }

    pub fn get_inner_mut(&mut self) -> (&mut R, &mut BytesMut) {
        (&mut self.reader, &mut self.buf)
    }

    pub async fn read_frame(&mut self) -> Result<Option<BytesMut>, ironrdp_pdu::RdpError>
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

    pub(crate) async fn decode_next_frame<F>(&mut self) -> Result<F, crate::RdpError>
    where
        F: Frame,
        R: Unpin,
    {
        let frame = self
            .read_frame()
            .await?
            .ok_or(crate::RdpError::UnexpectedStreamTermination)?;

        let item = F::decode(&frame[..])?;

        Ok(item)
    }
}

pub(crate) async fn encode_next_frame<W, F>(writer: &mut W, frame: F) -> Result<(), crate::RdpError>
where
    W: AsyncWrite + Unpin,
    F: Frame,
{
    let mut buf = BytesMut::new();
    let buf_writer = (&mut buf).writer();
    frame.encode(buf_writer)?;
    writer.write_all(&buf).await?;
    writer.flush().await?;
    Ok(())
}

/// Function to call when there are no more bytes available to be read from the underlying I/O.
fn decode_frame_eof(buf: &mut BytesMut) -> Result<Option<BytesMut>, ironrdp_pdu::RdpError> {
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
// TODO: try `&mut Bytes`
fn decode_frame(buf: &mut BytesMut) -> Result<Option<BytesMut>, ironrdp_pdu::RdpError> {
    let mut stream = buf.as_ref();
    if stream.is_empty() {
        return Ok(None);
    }
    let header = stream.read_u8()?;
    let action = header.get_bits(0..2);
    let action = Action::from_u8(action).ok_or(ironrdp_pdu::RdpError::InvalidActionCode(action))?;

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
