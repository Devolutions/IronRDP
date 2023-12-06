use std::io;
use std::marker::PhantomData;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Debug)]
pub struct TlsStream<S> {
    _marker: PhantomData<S>,
}

impl<S> AsyncRead for TlsStream<S> {
    fn poll_read(self: std::pin::Pin<&mut Self>, _: &mut Context<'_>, _: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl<S> AsyncWrite for TlsStream<S> {
    fn poll_write(self: std::pin::Pin<&mut Self>, _: &mut Context<'_>, _: &[u8]) -> Poll<Result<usize, io::Error>> {
        Poll::Ready(Ok(0))
    }

    fn poll_flush(self: std::pin::Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: std::pin::Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }
}

pub async fn upgrade<S>(stream: S, server_name: &str) -> io::Result<(TlsStream<S>, Vec<u8>)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    // Do nothing and fail
    let _ = (stream, server_name);
    Err(io::Error::other("no TLS backend enabled for this build"))
}
