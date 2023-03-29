use core::pin::Pin;
use core::task::{Context, Poll};
use std::io;

use futures_util::{pin_mut, ready, AsyncRead, AsyncWrite, Sink, Stream};
use gloo_net::websocket::futures::WebSocket;
use gloo_net::websocket::{Message as WebSocketMessage, WebSocketError};

pub(crate) struct WebSocketCompat {
    read_buf: Option<Vec<u8>>,
    inner: WebSocket,
}

impl WebSocketCompat {
    pub(crate) fn new(ws: WebSocket) -> Self {
        Self {
            read_buf: None,
            inner: ws,
        }
    }
}

impl AsyncRead for WebSocketCompat {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let read_buf = &mut this.read_buf;
        let inner = &mut this.inner;
        pin_mut!(inner);

        let mut data = if let Some(data) = read_buf.take() {
            data
        } else {
            match ready!(inner.as_mut().poll_next(cx)) {
                Some(Ok(m)) => match m {
                    WebSocketMessage::Text(s) => s.into_bytes(),
                    WebSocketMessage::Bytes(data) => data,
                },
                Some(Err(e)) => return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e.to_string()))),
                None => return Poll::Ready(Ok(0)),
            }
        };

        let bytes_to_copy = std::cmp::min(buf.len(), data.len());
        buf[..bytes_to_copy].copy_from_slice(&data[..bytes_to_copy]);

        if data.len() > bytes_to_copy {
            data.drain(..bytes_to_copy);
            *read_buf = Some(data);
        }

        Poll::Ready(Ok(bytes_to_copy))
    }
}

impl AsyncWrite for WebSocketCompat {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        macro_rules! try_in_poll {
            ($expr:expr) => {{
                match $expr {
                    Ok(o) => o,
                    // When using `AsyncWriteExt::write_all`, `io::ErrorKind::WriteZero` will be raised.
                    // In this case it means "attempted to write on a closed socket".
                    Err(WebSocketError::ConnectionClose(_)) => return Poll::Ready(Ok(0)),
                    Err(e) => return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e.to_string()))),
                }
            }};
        }

        let inner = &mut self.get_mut().inner;
        pin_mut!(inner);

        // try flushing preemptively
        let _ = inner.as_mut().poll_flush(cx);

        // make sure sink is ready to send
        try_in_poll!(ready!(inner.as_mut().poll_ready(cx)));

        // actually submit new item
        try_in_poll!(inner.as_mut().start_send(WebSocketMessage::Bytes(buf.to_vec())));
        // ^ if no error occurred, message is accepted and queued when calling `start_send`
        // (that is: `to_vec` is called only once)

        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner = &mut self.get_mut().inner;
        pin_mut!(inner);
        let res = ready!(inner.poll_flush(cx));
        Poll::Ready(websocket_to_io_result(res))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner = &mut self.get_mut().inner;
        pin_mut!(inner);
        let res = ready!(inner.poll_close(cx));
        Poll::Ready(websocket_to_io_result(res))
    }
}

fn websocket_to_io_result(res: Result<(), WebSocketError>) -> io::Result<()> {
    match res {
        Ok(()) => Ok(()),
        Err(WebSocketError::ConnectionClose(_)) => Ok(()),
        Err(e) => Err(io::Error::new(io::ErrorKind::Other, e.to_string())),
    }
}
