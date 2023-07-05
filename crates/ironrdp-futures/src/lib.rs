#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_async::*;

use std::io;
use std::pin::Pin;

use bytes::BytesMut;
use futures_util::io::{AsyncRead, AsyncWrite};

pub type FuturesFramed<S> = Framed<FuturesStream<S>>;

pub struct FuturesStream<S> {
    inner: S,
}

impl<S> StreamWrapper for FuturesStream<S> {
    type InnerStream = S;

    fn from_inner(stream: Self::InnerStream) -> Self {
        Self { inner: stream }
    }

    fn into_inner(self) -> Self::InnerStream {
        self.inner
    }

    fn get_inner(&self) -> &Self::InnerStream {
        &self.inner
    }

    fn get_inner_mut(&mut self) -> &mut Self::InnerStream {
        &mut self.inner
    }
}

impl<S> FramedRead for FuturesStream<S>
where
    S: Send + Sync + Unpin + AsyncRead,
{
    type ReadFut<'read> = Pin<Box<dyn std::future::Future<Output = io::Result<usize>> + Send + Sync + 'read>>
    where
        Self: 'read;

    fn read<'a>(&'a mut self, buf: &'a mut BytesMut) -> Self::ReadFut<'a> {
        use futures_util::io::AsyncReadExt as _;

        Box::pin(async {
            // NOTE(perf): tokio implementation is more efficient
            let mut read_bytes = [0u8; 1024];
            let len = self.inner.read(&mut read_bytes).await?;
            buf.extend_from_slice(&read_bytes[..len]);

            Ok(len)
        })
    }
}

impl<S> FramedWrite for FuturesStream<S>
where
    S: Send + Sync + Unpin + AsyncWrite,
{
    type WriteAllFut<'write> = Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + Sync + 'write>>
    where
        Self: 'write;

    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a> {
        use futures_util::io::AsyncWriteExt as _;

        Box::pin(async {
            self.inner.write_all(buf).await?;
            self.inner.flush().await?;

            Ok(())
        })
    }
}

pub type SingleThreadedFuturesFramed<S> = Framed<SingleThreadedFuturesStream<S>>;

pub struct SingleThreadedFuturesStream<S> {
    inner: S,
}

impl<S> StreamWrapper for SingleThreadedFuturesStream<S> {
    type InnerStream = S;

    fn from_inner(stream: Self::InnerStream) -> Self {
        Self { inner: stream }
    }

    fn into_inner(self) -> Self::InnerStream {
        self.inner
    }

    fn get_inner(&self) -> &Self::InnerStream {
        &self.inner
    }

    fn get_inner_mut(&mut self) -> &mut Self::InnerStream {
        &mut self.inner
    }
}

impl<S> FramedRead for SingleThreadedFuturesStream<S>
where
    S: Unpin + AsyncRead,
{
    type ReadFut<'read> = Pin<Box<dyn std::future::Future<Output = io::Result<usize>> + 'read>>
    where
        Self: 'read;

    fn read<'a>(&'a mut self, buf: &'a mut BytesMut) -> Self::ReadFut<'a> {
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

impl<S> FramedWrite for SingleThreadedFuturesStream<S>
where
    S: Unpin + AsyncWrite,
{
    type WriteAllFut<'write> = Pin<Box<dyn std::future::Future<Output = io::Result<()>> + 'write>>
    where
        Self: 'write;

    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a> {
        use futures_util::io::AsyncWriteExt as _;

        Box::pin(async {
            self.inner.write_all(buf).await?;
            self.inner.flush().await?;

            Ok(())
        })
    }
}
