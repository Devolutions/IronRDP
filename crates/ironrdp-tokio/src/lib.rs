#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_async::*;

use std::io;
use std::pin::Pin;

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite};

pub type TokioFramed<S> = Framed<TokioStream<S>>;

pub struct TokioStream<S> {
    inner: S,
}

impl<S> StreamWrapper for TokioStream<S> {
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

impl<S> FramedRead for TokioStream<S>
where
    S: Send + Sync + Unpin + AsyncRead,
{
    type ReadFut<'read> = Pin<Box<dyn std::future::Future<Output = io::Result<usize>> + Send + Sync + 'read>>
    where
        Self: 'read;

    fn read<'a>(&'a mut self, buf: &'a mut BytesMut) -> Self::ReadFut<'a> {
        use tokio::io::AsyncReadExt as _;

        Box::pin(async { self.inner.read_buf(buf).await })
    }
}

impl<S> FramedWrite for TokioStream<S>
where
    S: Send + Sync + Unpin + AsyncWrite,
{
    type WriteAllFut<'write> = Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + Sync + 'write>>
    where
        Self: 'write;

    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a> {
        use tokio::io::AsyncWriteExt as _;

        Box::pin(async {
            self.inner.write_all(buf).await?;
            self.inner.flush().await?;

            Ok(())
        })
    }
}

pub type LocalTokioFramed<S> = Framed<LocalTokioStream<S>>;

pub struct LocalTokioStream<S> {
    inner: S,
}

impl<S> StreamWrapper for LocalTokioStream<S> {
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

impl<S> FramedRead for LocalTokioStream<S>
where
    S: Unpin + AsyncRead,
{
    type ReadFut<'read> = Pin<Box<dyn std::future::Future<Output = io::Result<usize>> + 'read>>
    where
        Self: 'read;

    fn read<'a>(&'a mut self, buf: &'a mut BytesMut) -> Self::ReadFut<'a> {
        use tokio::io::AsyncReadExt as _;

        Box::pin(async { self.inner.read_buf(buf).await })
    }
}

impl<S> FramedWrite for LocalTokioStream<S>
where
    S: Unpin + AsyncWrite,
{
    type WriteAllFut<'write> = Pin<Box<dyn std::future::Future<Output = io::Result<()>> + 'write>>
    where
        Self: 'write;

    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a> {
        use tokio::io::AsyncWriteExt as _;

        Box::pin(async {
            self.inner.write_all(buf).await?;
            self.inner.flush().await?;

            Ok(())
        })
    }
}
