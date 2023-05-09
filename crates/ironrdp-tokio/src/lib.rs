use std::io;
use std::pin::Pin;

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite};

pub use ironrdp_async::*;

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

impl<S> FramedWrite for TokioStream<S>
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
