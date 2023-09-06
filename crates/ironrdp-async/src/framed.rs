use std::io;

use bytes::{Bytes, BytesMut};
use ironrdp_pdu::PduHint;

// TODO: investigate if we could use static async fn / return position impl trait in traits when stabilized:
// https://github.com/rust-lang/rust/issues/91611

pub trait FramedRead {
    type ReadFut<'read>: std::future::Future<Output = io::Result<usize>> + 'read
    where
        Self: 'read;

    /// Reads from stream and fills internal buffer
    fn read<'a>(&'a mut self, buf: &'a mut BytesMut) -> Self::ReadFut<'a>;
}

pub trait FramedWrite {
    type WriteAllFut<'write>: std::future::Future<Output = io::Result<()>> + 'write
    where
        Self: 'write;

    /// Writes an entire buffer into this stream.
    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a>;
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
    /// Reads from stream and fills internal buffer
    pub async fn read(&mut self) -> io::Result<usize> {
        self.stream.read(&mut self.buf).await
    }

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
}

impl<S> Framed<S>
where
    S: FramedWrite,
{
    /// Writes an entire buffer into this stream.
    pub async fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.stream.write_all(buf).await
    }
}
