//! AsyncRead/AsyncWrite adapter bridging the driver task to TLS.
//!
//! The driver owns the `RdpeudpConnection` and `UdpSocket`. This module
//! provides `RdpeudpStream`, which tokio-rustls wraps for TLS. Two ring
//! buffers behind `Arc<Mutex<SharedIo>>` shuttle bytes between the driver
//! and the TLS layer without requiring the TLS layer to know it's running
//! over UDP.

use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::io;
use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// How many undelivered bytes may pile up in `SharedIo::write_buf` before
/// `AsyncWrite::poll_write` stops accepting more.
///
/// Nothing else bounds it. `RdpeudpConnection::send`'s own `SendBufferFull`
/// only rejects one call's worth of bytes; without this, a caller that keeps
/// writing through that rejection (the pattern `queue_write`'s retry loop is
/// built for) has nowhere for `write_buf` to stop growing, for as long as
/// congestion or an unresponsive peer keeps the send buffer full. Mirrors
/// `driver::READ_BUF_HIGH_WATER`'s reasoning on the other direction.
pub(crate) const WRITE_BUF_HIGH_WATER: usize = 1 << 20;

// ════════════════════════════════════════════════════════════════════
// SharedIo
// ════════════════════════════════════════════════════════════════════

/// Shared state between the driver task and the `RdpeudpStream`.
///
/// The driver deposits decrypted RDPEUDP2 payload bytes into `read_buf`
/// (waking the TLS reader), and picks up plaintext bytes from `write_buf`
/// (placed there by TLS) to feed into `conn.send()`.
pub(crate) struct SharedIo {
    /// Bytes available for `AsyncRead` consumers (driver writes here
    /// when `Event::DataReceived` fires).
    pub(crate) read_buf: BytesMut,

    /// Waker registered by `AsyncRead::poll_read` when `read_buf` is empty.
    pub(crate) read_waker: Option<Waker>,

    /// Bytes queued for transmission. `AsyncWrite::poll_write` appends
    /// here; the driver drains this into `conn.send()`.
    pub(crate) write_buf: BytesMut,

    /// Waker registered by the driver when it finds `write_buf` empty.
    /// Fired by `poll_write` after depositing data.
    pub(crate) write_waker: Option<Waker>,

    /// Waker registered by `poll_flush` when `write_buf` is non-empty.
    /// Fired by the driver after draining `write_buf`, so the flusher
    /// can re-check and return `Ready`.
    ///
    /// Separate from `write_waker` because the two have opposite
    /// trigger conditions (data-present vs data-drained).
    pub(crate) flush_waker: Option<Waker>,

    /// Waker registered by the driver when `read_buf` has grown past what it
    /// is willing to hold. Fired by `poll_read` once it has taken bytes out.
    ///
    /// Without it the driver has no way to learn that the consumer caught up,
    /// and would sit throttled until some unrelated timer happened to fire.
    pub(crate) read_drained_waker: Option<Waker>,

    /// Waker registered by `poll_write` when `write_buf` is at or over
    /// `WRITE_BUF_HIGH_WATER`. Fired by the driver once it drains `write_buf`
    /// into `conn.send()`.
    ///
    /// Without it a writer blocked on a full `write_buf` has no way to learn
    /// the driver made room, and would stay pending until some unrelated
    /// wake happened to reach it.
    pub(crate) write_room_waker: Option<Waker>,

    /// Fatal error from the driver (propagated to reads/writes).
    pub(crate) error: Option<io::ErrorKind>,

    /// Set when the RDPEUDP2 connection has been cleanly shut down.
    pub(crate) closed: bool,
}

impl SharedIo {
    pub(crate) fn new() -> Self {
        Self {
            read_buf: BytesMut::with_capacity(8192),
            read_waker: None,
            write_buf: BytesMut::with_capacity(8192),
            write_waker: None,
            flush_waker: None,
            read_drained_waker: None,
            write_room_waker: None,
            error: None,
            closed: false,
        }
    }

    /// Mark the connection closed and wake every task that might be
    /// blocked on this state, so a close is never missed by a task that
    /// was mid-`Pending` when it happened.
    ///
    /// The five call sites that used to set `closed = true` directly each
    /// hand-picked which wakers to fire, and drifted out of sync with each
    /// other: two never woke `flush_waker` (a `poll_flush` in flight when
    /// the connection self-closed, cleanly or via `Event::ConnectionClosed`,
    /// hung forever), and two never woke `read_drained_waker` (the driver
    /// itself, throttled on its own full `read_buf`, would never notice an
    /// externally-initiated shutdown). Centralizing the wake list here means
    /// a waker added for one reason automatically gets covered everywhere
    /// closing can happen, rather than needing five separate updates found
    /// by tracing each call site's `Future` by hand.
    pub(crate) fn close(&mut self) {
        self.closed = true;
        for waker in [
            self.read_waker.take(),
            self.write_waker.take(),
            self.flush_waker.take(),
            self.read_drained_waker.take(),
            self.write_room_waker.take(),
        ]
        .into_iter()
        .flatten()
        {
            waker.wake();
        }
    }
}

// ════════════════════════════════════════════════════════════════════
// RdpeudpStream
// ════════════════════════════════════════════════════════════════════

/// Async byte-stream adapter over an RDPEUDP2 connection.
///
/// Implements `AsyncRead + AsyncWrite + Unpin` so tokio-rustls can
/// wrap it for TLS. The stream itself doesn't touch the network; the
/// driver task does all UDP I/O and state-machine driving.
pub(crate) struct RdpeudpStream {
    pub(crate) shared: Arc<Mutex<SharedIo>>,
}

impl RdpeudpStream {
    pub(crate) fn new(shared: Arc<Mutex<SharedIo>>) -> Self {
        Self { shared }
    }
}

impl AsyncRead for RdpeudpStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let mut shared = self
            .shared
            .lock()
            .map_err(|_| io::Error::other("shared lock poisoned"))?;

        if !shared.read_buf.is_empty() {
            let n = core::cmp::min(buf.remaining(), shared.read_buf.len());
            buf.put_slice(&shared.read_buf.split_to(n));

            // Tell the driver it has room again, in case it stopped taking
            // packets off the socket while this buffer was full.
            if let Some(waker) = shared.read_drained_waker.take() {
                waker.wake();
            }

            return Poll::Ready(Ok(()));
        }

        if let Some(kind) = shared.error {
            return Poll::Ready(Err(io::Error::new(kind, "RDPEUDP2 transport error")));
        }

        if shared.closed {
            // EOF
            return Poll::Ready(Ok(()));
        }

        // No data available: register waker so the driver can wake us
        shared.read_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl AsyncWrite for RdpeudpStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let mut shared = self
            .shared
            .lock()
            .map_err(|_| io::Error::other("shared lock poisoned"))?;

        if let Some(kind) = shared.error {
            return Poll::Ready(Err(io::Error::new(kind, "RDPEUDP2 transport error")));
        }

        if shared.closed {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "connection closed")));
        }

        if shared.write_buf.len() >= WRITE_BUF_HIGH_WATER {
            // The driver isn't keeping up (or the peer isn't acking): stop
            // accepting more instead of letting write_buf grow without
            // bound. The driver wakes this once it drains write_buf.
            shared.write_room_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }

        shared.write_buf.extend_from_slice(buf);

        // Wake the driver so it picks up the new data
        if let Some(waker) = shared.write_waker.take() {
            waker.wake();
        }

        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut shared = self
            .shared
            .lock()
            .map_err(|_| io::Error::other("shared lock poisoned"))?;

        if let Some(kind) = shared.error {
            return Poll::Ready(Err(io::Error::new(kind, "RDPEUDP2 transport error")));
        }

        if shared.write_buf.is_empty() {
            return Poll::Ready(Ok(()));
        }

        // Data still pending: register on flush_waker (separate from
        // write_waker, which the driver uses to detect new data).
        // The driver wakes flush_waker after draining write_buf.
        shared.flush_waker = Some(cx.waker().clone());
        Poll::Pending
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut shared = self
            .shared
            .lock()
            .map_err(|_| io::Error::other("shared lock poisoned"))?;

        shared.close();

        Poll::Ready(Ok(()))
    }
}

// Unpin is auto-derived because RdpeudpStream contains only Arc (no self-referential state).

#[cfg(test)]
mod tests {
    use std::task::Wake;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    /// Minimal waker that tracks whether it was woken.
    struct TestWaker {
        woken: core::sync::atomic::AtomicBool,
    }

    impl TestWaker {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                woken: core::sync::atomic::AtomicBool::new(false),
            })
        }

        fn was_woken(&self) -> bool {
            self.woken.load(core::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Wake for TestWaker {
        fn wake(self: Arc<Self>) {
            self.woken.store(true, core::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn shared_io_starts_empty() {
        let shared = SharedIo::new();
        assert!(shared.read_buf.is_empty());
        assert!(shared.write_buf.is_empty());
        assert!(!shared.closed);
        assert!(shared.error.is_none());
    }

    #[test]
    fn write_deposits_into_write_buf_and_wakes_driver() {
        let driver_waker = TestWaker::new();

        let shared = Arc::new(Mutex::new(SharedIo::new()));
        {
            let mut s = shared.lock().expect("lock");
            s.write_waker = Some(Waker::from(Arc::clone(&driver_waker)));
        }

        let mut stream = RdpeudpStream::new(Arc::clone(&shared));

        let rt = tokio::runtime::Builder::new_current_thread().build().expect("rt");
        rt.block_on(async {
            stream.write_all(b"hello").await.expect("write");
        });

        let s = shared.lock().expect("lock");
        assert_eq!(&*s.write_buf, b"hello");
        assert!(driver_waker.was_woken());
    }

    #[test]
    fn read_returns_data_deposited_by_driver() {
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        {
            let mut s = shared.lock().expect("lock");
            s.read_buf.extend_from_slice(b"world");
        }

        let mut stream = RdpeudpStream::new(shared);

        let rt = tokio::runtime::Builder::new_current_thread().build().expect("rt");
        rt.block_on(async {
            let mut buf = [0u8; 32];
            let n = stream.read(&mut buf).await.expect("read");
            assert_eq!(&buf[..n], b"world");
        });
    }

    #[test]
    fn read_returns_eof_when_closed() {
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        {
            let mut s = shared.lock().expect("lock");
            s.closed = true;
        }

        let mut stream = RdpeudpStream::new(shared);

        let rt = tokio::runtime::Builder::new_current_thread().build().expect("rt");
        rt.block_on(async {
            let mut buf = [0u8; 32];
            let n = stream.read(&mut buf).await.expect("read");
            assert_eq!(n, 0);
        });
    }

    #[test]
    fn write_fails_when_closed() {
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        {
            let mut s = shared.lock().expect("lock");
            s.closed = true;
        }

        let mut stream = RdpeudpStream::new(shared);

        let rt = tokio::runtime::Builder::new_current_thread().build().expect("rt");
        rt.block_on(async {
            let result = stream.write_all(b"nope").await;
            assert!(result.is_err());
        });
    }

    #[test]
    fn read_registers_waker_when_empty() {
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        let mut stream = RdpeudpStream::new(Arc::clone(&shared));

        let test_waker = TestWaker::new();
        let waker = Waker::from(Arc::clone(&test_waker));
        let mut cx = Context::from_waker(&waker);

        let mut buf = [0u8; 32];
        let mut read_buf = ReadBuf::new(&mut buf);
        let result = Pin::new(&mut stream).poll_read(&mut cx, &mut read_buf);
        assert!(result.is_pending());

        // Now the driver deposits data and wakes the reader
        {
            let mut s = shared.lock().expect("lock");
            s.read_buf.extend_from_slice(b"data");
            if let Some(w) = s.read_waker.take() {
                w.wake();
            }
        }

        assert!(test_waker.was_woken());
    }

    #[test]
    fn read_propagates_error() {
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        {
            let mut s = shared.lock().expect("lock");
            s.error = Some(io::ErrorKind::ConnectionReset);
        }

        let mut stream = RdpeudpStream::new(shared);

        let rt = tokio::runtime::Builder::new_current_thread().build().expect("rt");
        rt.block_on(async {
            let mut buf = [0u8; 32];
            let result = stream.read(&mut buf).await;
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionReset);
        });
    }

    /// This is the actual defect: without a bound, `write_buf` had nothing
    /// stopping it from growing without limit once `SendBufferFull` stopped
    /// being fatal and became a retried condition instead. `poll_write` must
    /// refuse once `write_buf` is full, the same way the read side already
    /// stops pulling off the socket once `read_buf` is full.
    #[test]
    fn poll_write_blocks_once_write_buf_is_full() {
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        {
            let mut s = shared.lock().expect("lock");
            s.write_buf.extend_from_slice(&vec![0u8; WRITE_BUF_HIGH_WATER]);
        }

        let mut stream = RdpeudpStream::new(Arc::clone(&shared));

        let test_waker = TestWaker::new();
        let waker = Waker::from(Arc::clone(&test_waker));
        let mut cx = Context::from_waker(&waker);

        let result = Pin::new(&mut stream).poll_write(&mut cx, b"one more byte");
        assert!(result.is_pending(), "poll_write should block at the high-water mark");

        let s = shared.lock().expect("lock");
        assert_eq!(
            s.write_buf.len(),
            WRITE_BUF_HIGH_WATER,
            "the blocked write must not have been appended"
        );
        assert!(!test_waker.was_woken(), "not woken yet, nothing has drained write_buf");
    }

    /// The driver wakes a blocked writer once it drains `write_buf`, the
    /// same relationship `read_drained_waker` has on the read side.
    #[test]
    fn write_room_waker_fires_once_write_buf_drains() {
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        {
            let mut s = shared.lock().expect("lock");
            s.write_buf.extend_from_slice(&vec![0u8; WRITE_BUF_HIGH_WATER]);
        }

        let mut stream = RdpeudpStream::new(Arc::clone(&shared));

        let test_waker = TestWaker::new();
        let waker = Waker::from(Arc::clone(&test_waker));
        let mut cx = Context::from_waker(&waker);

        let result = Pin::new(&mut stream).poll_write(&mut cx, b"blocked");
        assert!(result.is_pending());

        // The driver drains write_buf (mirrors what branch 2 does) and wakes
        // the blocked writer, the same way it already does for flush_waker.
        {
            let mut s = shared.lock().expect("lock");
            s.write_buf.clear();
            if let Some(w) = s.write_room_waker.take() {
                w.wake();
            }
        }

        assert!(test_waker.was_woken());
    }

    /// This is the second defect the same review round found: the five call
    /// sites that used to set `closed = true` directly had each hand-picked
    /// which wakers to fire, and two of them never woke `flush_waker` (a
    /// `poll_flush` in flight would have hung on a clean self-close) while
    /// two others never woke `read_drained_waker` (the driver itself,
    /// throttled on its own full `read_buf`, would never have noticed an
    /// externally-initiated shutdown). `close()` must wake every one of
    /// them, unconditionally, so no call site can drift out of sync again.
    #[test]
    fn close_wakes_every_registered_waker() {
        let mut shared = SharedIo::new();

        let read_waker = TestWaker::new();
        let write_waker = TestWaker::new();
        let flush_waker = TestWaker::new();
        let read_drained_waker = TestWaker::new();
        let write_room_waker = TestWaker::new();

        shared.read_waker = Some(Waker::from(Arc::clone(&read_waker)));
        shared.write_waker = Some(Waker::from(Arc::clone(&write_waker)));
        shared.flush_waker = Some(Waker::from(Arc::clone(&flush_waker)));
        shared.read_drained_waker = Some(Waker::from(Arc::clone(&read_drained_waker)));
        shared.write_room_waker = Some(Waker::from(Arc::clone(&write_room_waker)));

        shared.close();

        assert!(shared.closed);
        assert!(read_waker.was_woken(), "read_waker not woken");
        assert!(write_waker.was_woken(), "write_waker not woken");
        assert!(flush_waker.was_woken(), "flush_waker not woken");
        assert!(read_drained_waker.was_woken(), "read_drained_waker not woken");
        assert!(write_room_waker.was_woken(), "write_room_waker not woken");
    }
}
