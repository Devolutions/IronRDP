#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://webdevolutions.blob.core.windows.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg"
)]

#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_async::*;

use std::io;
use std::pin::Pin;

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf};

pub type TokioFramed<S> = Framed<TokioStream<S>>;

pub fn split_tokio_framed<S>(framed: TokioFramed<S>) -> (TokioFramed<ReadHalf<S>>, TokioFramed<WriteHalf<S>>)
where
    S: Sync + Unpin + AsyncRead + AsyncWrite,
{
    let (stream, leftover) = framed.into_inner();
    let (read_half, write_half) = tokio::io::split(stream);
    let framed_read = TokioFramed::new_with_leftover(read_half, leftover);
    let framed_write = TokioFramed::new(write_half);
    (framed_read, framed_write)
}

pub fn unsplit_tokio_framed<S>(reader: TokioFramed<ReadHalf<S>>, writer: TokioFramed<WriteHalf<S>>) -> TokioFramed<S>
where
    S: Sync + Unpin + AsyncRead + AsyncWrite,
{
    let (reader, leftover) = reader.into_inner();
    let writer = writer.into_inner_no_leftover();
    TokioFramed::new_with_leftover(reader.unsplit(writer), leftover)
}

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
