use std::io;
use std::pin::Pin;

use bytes::{Bytes, BytesMut};
use ironrdp_pdu::PduHint;

// TODO: use static async fn / return position impl trait in traits where stabiziled (https://github.com/rust-lang/rust/issues/91611)

pub trait FramedRead: private::Sealed {
    /// Reads from stream and fills internal buffer
    fn read<'a>(
        &'a mut self,
        buf: &'a mut BytesMut,
    ) -> Pin<Box<dyn std::future::Future<Output = io::Result<usize>> + 'a>>
    where
        Self: 'a;
}

pub trait FramedWrite: private::Sealed {
    /// Writes an entire buffer into this stream.
    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Pin<Box<dyn std::future::Future<Output = io::Result<()>> + 'a>>
    where
        Self: 'a;
}

pub struct Framed<S> {
    stream: S,
    buf: BytesMut,
}

impl<S> Framed<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: BytesMut::new(),
        }
    }

    pub fn into_inner(self) -> (S, BytesMut) {
        (self.stream, self.buf)
    }

    pub fn into_inner_no_leftover(self) -> S {
        let (stream, leftover) = self.into_inner();
        debug_assert_eq!(leftover.len(), 0, "unexpected leftover");
        stream
    }

    pub fn get_inner(&self) -> (&S, &BytesMut) {
        (&self.stream, &self.buf)
    }

    pub fn get_inner_mut(&mut self) -> (&mut S, &mut BytesMut) {
        (&mut self.stream, &mut self.buf)
    }

    pub fn peek(&self) -> &[u8] {
        &self.buf
    }
}

#[cfg(feature = "tokio")]
impl<S> Framed<TokioCompat<S>> {
    pub fn tokio_new(stream: S) -> Self {
        Self {
            stream: TokioCompat { inner: stream },
            buf: BytesMut::new(),
        }
    }

    pub fn tokio_into_inner(self) -> (S, BytesMut) {
        (self.stream.inner, self.buf)
    }

    pub fn tokio_into_inner_no_leftover(self) -> S {
        let (stream, leftover) = self.tokio_into_inner();
        assert_eq!(leftover.len(), 0, "unexpected leftover");
        stream
    }

    pub fn tokio_get_inner(&self) -> (&S, &BytesMut) {
        (&self.stream.inner, &self.buf)
    }

    pub fn tokio_get_inner_mut(&mut self) -> (&mut S, &mut BytesMut) {
        (&mut self.stream.inner, &mut self.buf)
    }
}

#[cfg(feature = "futures")]
impl<S> Framed<FuturesCompat<S>> {
    pub fn futures_new(stream: S) -> Self {
        Self {
            stream: FuturesCompat { inner: stream },
            buf: BytesMut::new(),
        }
    }

    pub fn futures_into_inner(self) -> (S, BytesMut) {
        (self.stream.inner, self.buf)
    }

    pub fn futures_into_inner_no_leftover(self) -> S {
        let (stream, leftover) = self.futures_into_inner();
        debug_assert_eq!(leftover.len(), 0, "unexpected leftover");
        stream
    }

    pub fn futures_get_inner(&self) -> (&S, &BytesMut) {
        (&self.stream.inner, &self.buf)
    }

    pub fn futures_get_inner_mut(&mut self) -> (&mut S, &mut BytesMut) {
        (&mut self.stream.inner, &mut self.buf)
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

    pub async fn read_exact(&mut self, length: usize) -> io::Result<Bytes> {
        loop {
            if self.buf.len() >= length {
                return Ok(self.buf.split_to(length).freeze());
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

    pub async fn read_pdu(&mut self) -> io::Result<(ironrdp_pdu::Action, Bytes)> {
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
                    return self.read_exact(length).await;
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
    /// Reads from stream and fills internal buffer
    pub async fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.stream.write_all(buf).await
    }
}

#[cfg(feature = "tokio")]
pub struct TokioCompat<S> {
    inner: S,
}

#[cfg(feature = "tokio")]
mod tokio_impl {
    use tokio::io::{AsyncRead, AsyncWrite};

    use super::*;

    impl<S> private::Sealed for TokioCompat<S> {}

    impl<S> FramedRead for TokioCompat<S>
    where
        S: Unpin + AsyncRead,
    {
        fn read<'a>(
            &'a mut self,
            buf: &'a mut BytesMut,
        ) -> Pin<Box<dyn std::future::Future<Output = io::Result<usize>> + 'a>>
        where
            Self: 'a,
        {
            use tokio::io::AsyncReadExt as _;

            Box::pin(async { self.inner.read_buf(buf).await })
        }
    }

    impl<S> FramedWrite for TokioCompat<S>
    where
        S: Unpin + AsyncWrite,
    {
        fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Pin<Box<dyn std::future::Future<Output = io::Result<()>> + 'a>>
        where
            Self: 'a,
        {
            use tokio::io::AsyncWriteExt as _;

            Box::pin(async {
                self.inner.write_all(buf).await?;
                self.inner.flush().await?;

                Ok(())
            })
        }
    }
}

#[cfg(feature = "futures")]
pub struct FuturesCompat<S> {
    pub(super) inner: S,
}

#[cfg(feature = "futures")]
mod futures_impl {
    pub use futures_util::io::{AsyncRead, AsyncWrite};

    use super::*;

    impl<S> private::Sealed for FuturesCompat<S> {}

    impl<S> FramedRead for FuturesCompat<S>
    where
        S: Unpin + AsyncRead,
    {
        fn read<'a>(
            &'a mut self,
            buf: &'a mut BytesMut,
        ) -> Pin<Box<dyn std::future::Future<Output = io::Result<usize>> + 'a>>
        where
            Self: 'a,
        {
            use futures_util::io::AsyncReadExt as _;

            Box::pin(async {
                // NOTE(perf): tokio implementation is more efficient
                let mut read_bytes = [0u8; 1024];
                let len = self.inner.read(&mut read_bytes[..]).await?;
                buf.extend_from_slice(&read_bytes[..len]);

                Ok(len)
            })
        }
    }

    impl<S> FramedWrite for FuturesCompat<S>
    where
        S: Unpin + AsyncWrite,
    {
        fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Pin<Box<dyn std::future::Future<Output = io::Result<()>> + 'a>>
        where
            Self: 'a,
        {
            use futures_util::io::AsyncWriteExt as _;

            Box::pin(async {
                self.inner.write_all(buf).await?;
                self.inner.flush().await?;

                Ok(())
            })
        }
    }
}

mod private {
    pub trait Sealed {}
}
