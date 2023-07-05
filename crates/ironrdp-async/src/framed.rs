use std::io;
use std::pin::Pin;

use bytes::{Bytes, BytesMut};
use ironrdp_pdu::PduHint;

// TODO: use static async fn / return position impl trait in traits when stabiziled (https://github.com/rust-lang/rust/issues/91611)

pub trait FramedRead {
    /// Reads from stream and fills internal buffer
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// [`tokio::select!`](crate::select) statement and some other branch
    /// completes first, then it is guaranteed that no data was read.
    fn read<'a>(
        &'a mut self,
        buf: &'a mut BytesMut,
    ) -> Pin<Box<dyn std::future::Future<Output = io::Result<usize>> + 'a>>
    where
        Self: 'a;
}

pub trait FramedWrite {
    /// Writes an entire buffer into this stream.
    ///
    /// # Cancel safety
    ///
    /// This method is not cancellation safe. If it is used as the event
    /// in a [`tokio::select!`](crate::select) statement and some other
    /// branch completes first, then the provided buffer may have been
    /// partially written, but future calls to `write_all` will start over
    /// from the beginning of the buffer.
    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Pin<Box<dyn std::future::Future<Output = io::Result<()>> + 'a>>
    where
        Self: 'a;
}

pub trait StreamWrapper: Sized {
    type InnerStream;

    fn from_inner(stream: Self::InnerStream) -> Self;

    fn into_inner(self) -> Self::InnerStream;

    fn get_inner(&self) -> &Self::InnerStream;

    fn get_inner_mut(&mut self) -> &mut Self::InnerStream;
}

pub struct Framed<S> {
    stream: S,
    buf: BytesMut,
}

impl<S> Framed<S> {
    pub fn peek(&self) -> &[u8] {
        &self.buf
    }
}

impl<S> Framed<S>
where
    S: StreamWrapper,
{
    pub fn new(stream: S::InnerStream) -> Self {
        Self {
            stream: S::from_inner(stream),
            buf: BytesMut::new(),
        }
    }

    pub fn into_inner(self) -> (S::InnerStream, BytesMut) {
        (self.stream.into_inner(), self.buf)
    }

    pub fn into_inner_no_leftover(self) -> S::InnerStream {
        let (stream, leftover) = self.into_inner();
        debug_assert_eq!(leftover.len(), 0, "unexpected leftover");
        stream
    }

    pub fn get_inner(&self) -> (&S::InnerStream, &BytesMut) {
        (self.stream.get_inner(), &self.buf)
    }

    pub fn get_inner_mut(&mut self) -> (&mut S::InnerStream, &mut BytesMut) {
        (self.stream.get_inner_mut(), &mut self.buf)
    }
}

impl<S> Framed<S>
where
    S: FramedRead,
{
    /// Accumulates at least `length` bytes and returns exactly `length` bytes, keeping the leftover in the internal buffer.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// [`tokio::select!`](crate::select) statement and some other branch
    /// completes first, then it is safe to drop the future and re-create it later.
    /// Data may have been read, but it will be stored in the internal buffer.
    pub async fn read_exact(&mut self, length: usize) -> io::Result<BytesMut> {
        loop {
            if self.buf.len() >= length {
                return Ok(self.buf.split_to(length));
            } else {
                self.buf.reserve(length - self.buf.len());
            }

            let len = self.read().await?;

            // Handle EOF
            if len == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
            }
        }
    }

    /// Reads a standard RDP PDU frame.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// [`tokio::select!`](crate::select) statement and some other branch
    /// completes first, then it is safe to drop the future and re-create it later.
    /// Data may have been read, but it will be stored in the internal buffer.
    pub async fn read_pdu(&mut self) -> io::Result<(ironrdp_pdu::Action, BytesMut)> {
        loop {
            // Try decoding and see if a frame has been received already
            match ironrdp_pdu::find_size(self.peek()) {
                Ok(Some(pdu_info)) => {
                    let frame = self.read_exact(pdu_info.length).await?;

                    return Ok((pdu_info.action, frame));
                }
                Ok(None) => {
                    let len = self.read().await?;

                    // Handle EOF
                    if len == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
                    }
                }
                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
            };
        }
    }

    /// Reads a frame using the provided PduHint.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// [`tokio::select!`](crate::select) statement and some other branch
    /// completes first, then it is safe to drop the future and re-create it later.
    /// Data may have been read, but it will be stored in the internal buffer.
    pub async fn read_by_hint(&mut self, hint: &dyn PduHint) -> io::Result<Bytes> {
        loop {
            match hint
                .find_size(self.peek())
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            {
                Some(length) => {
                    return Ok(self.read_exact(length).await?.freeze());
                }
                None => {
                    let len = self.read().await?;

                    // Handle EOF
                    if len == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
                    }
                }
            };
        }
    }

    /// Reads from stream and fills internal buffer, returning how many bytes were read.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// [`tokio::select!`](crate::select) statement and some other branch
    /// completes first, then it is guaranteed that no data was read.
    async fn read(&mut self) -> io::Result<usize> {
        self.stream.read(&mut self.buf).await
    }
}

impl<S> Framed<S>
where
    S: FramedWrite,
{
    /// Attempts to write an entire buffer into this `Framed`’s stream.
    ///
    /// # Cancel safety
    ///
    /// This method is not cancellation safe. If it is used as the event
    /// in a [`tokio::select!`](crate::select) statement and some other
    /// branch completes first, then the provided buffer may have been
    /// partially written, but future calls to `write_all` will start over
    /// from the beginning of the buffer.
    pub async fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.stream.write_all(buf).await
    }
}
