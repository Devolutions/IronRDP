use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::io;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::{CertificateValidation, CertificateValidationCallback};

#[derive(Debug)]
pub struct TlsStream<S> {
    _marker: PhantomData<S>,
}

impl<S> AsyncRead for TlsStream<S> {
    fn poll_read(self: Pin<&mut Self>, _: &mut Context<'_>, _: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl<S> AsyncWrite for TlsStream<S> {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, _: &[u8]) -> Poll<Result<usize, io::Error>> {
        Poll::Ready(Ok(0))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }
}

pub async fn upgrade<S>(stream: S, server_name: &str) -> io::Result<(TlsStream<S>, Vec<u8>)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    upgrade_with_certificate_validation(stream, server_name, CertificateValidation::default()).await
}

/// The stub backend performs no handshake regardless of the requested policy.
pub async fn upgrade_with_certificate_validation<S>(
    stream: S,
    server_name: &str,
    certificate_validation: CertificateValidation,
) -> io::Result<(TlsStream<S>, Vec<u8>)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let _ = (stream, server_name, certificate_validation);
    Err(io::Error::other("no TLS backend enabled for this build"))
}

/// The stub backend cannot perform a certificate-validation callback.
pub async fn upgrade_with_certificate_validation_callback<S>(
    stream: S,
    server_name: &str,
    callback: CertificateValidationCallback,
) -> io::Result<(TlsStream<S>, Vec<u8>)>
where
    S: Unpin + AsyncRead + AsyncWrite,
{
    let _ = (stream, server_name, callback);
    Err(io::Error::other("no TLS backend enabled for this build"))
}

/// The stub backend performs no handshake and reports nothing.
pub fn negotiated<S>(_stream: &TlsStream<S>) -> crate::NegotiatedTls {
    crate::NegotiatedTls::default()
}
