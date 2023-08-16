use std::io;
use std::pin::Pin;

use bytes::BytesMut;
use futures_util::io::{AsyncRead, AsyncWrite};

#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_async::*;

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

impl<S> FramedWrite for FuturesStream<S>
where
    S: Unpin + AsyncWrite,
{
    fn write_all<'a>(
        &'a mut self,
        buf: &'a [u8],
    ) -> Pin<Box<dyn std::future::Future<Output = io::Result<()>> + 'a + Send>>
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
