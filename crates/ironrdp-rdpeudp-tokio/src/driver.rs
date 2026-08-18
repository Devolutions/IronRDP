//! Driver task: the async event loop bridging UDP socket ↔ sans-I/O state machine.
//!
//! A single `tokio::spawn`'d task owns the `UdpSocket` and `RdpeudpConnection`,
//! running a `select!` loop over three event sources:
//!
//! 1. **UDP recv**: incoming datagrams from the network
//! 2. **Write data**: the TLS layer has plaintext to transmit
//! 3. **Timer**: RDPEUDP2 retransmit / ACK-delay / keep-alive / idle timeouts
//!
//! The driver communicates with the `RdpeudpStream` (which tokio-rustls wraps)
//! through `Arc<Mutex<SharedIo>>`.

use crate::clock::Clock;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::io;
use std::sync::{Arc, Mutex};

use ironrdp_rdpeudp::pdu::{V1Datagram, V1Flags};
use ironrdp_rdpeudp::{Event, RdpeudpConnection, RdpeudpError, RdpeudpErrorKind, SendError};
use tokio::net::UdpSocket;
use tokio::sync::Notify;

use crate::error::{DriverError, DriverErrorExt as _, DriverErrorKind};
use crate::stream::SharedIo;

/// Maximum UDP datagram size we'll attempt to receive.
/// RDPEUDP2 typically negotiates 1232-byte MTU, but we budget
/// a full Ethernet jumbo frame for safety.
const RECV_BUF_SIZE: usize = 9000;

/// How many undelivered bytes may pile up in `SharedIo::read_buf` before the
/// driver stops taking packets off the socket.
///
/// Nothing else bounds it. The receive window limits how much the peer may
/// have in flight, but the driver drains that window into `read_buf` on every
/// pass, so a peer sending faster than the TLS and tunnel layers consume grows
/// it without limit and the process runs out of memory.
///
/// Ceasing to read is real backpressure rather than a dropped-on-the-floor
/// policy: unread datagrams stay in the socket buffer, our acknowledgements
/// stop, and [MS-RDPEUDP2] 3.1.1.2.2's receive window closes on the sender,
/// which is the protocol's own way of saying "wait".
const READ_BUF_HIGH_WATER: usize = 1 << 20;

// ════════════════════════════════════════════════════════════════════
// Driver
// ════════════════════════════════════════════════════════════════════

/// The driver task's internal state.
pub(crate) struct Driver {
    socket: UdpSocket,
    conn: RdpeudpConnection,
    shared: Arc<Mutex<SharedIo>>,
    /// Notified by the driver when Event::Connected fires.
    connected_notify: Arc<Notify>,
    /// Whether Event::Connected has already been signaled.
    connected_signaled: bool,
    /// Receive buffer (reused across iterations).
    recv_buf: Vec<u8>,
    /// The only clock in the stack; the state machine has none.
    clock: Clock,
    /// Data `RdpeudpConnection::send` rejected with `SendBufferFull`, held
    /// here to retry once the send queue has room again instead of being
    /// dropped or treated as a fatal error.
    pending_write: Option<Vec<u8>>,
}

impl Driver {
    pub(crate) fn new(
        socket: UdpSocket,
        conn: RdpeudpConnection,
        shared: Arc<Mutex<SharedIo>>,
        connected_notify: Arc<Notify>,
    ) -> Self {
        Self {
            socket,
            conn,
            shared,
            connected_notify,
            connected_signaled: false,
            recv_buf: vec![0u8; RECV_BUF_SIZE],
            clock: Clock::new(),
            pending_write: None,
        }
    }

    /// Run the driver event loop until the connection closes or errors.
    pub(crate) async fn run(mut self) -> Result<(), DriverError> {
        let result = self.run_to_completion().await;

        // However this ended, the stream side has to hear about it. A reader
        // parked in `poll_read` has left its waker here and nothing else will
        // poll it, so returning an error without coming through here hangs
        // that task for the life of the process.
        self.release_stream(result.as_ref().err());

        result
    }

    /// Mark the shared state closed and wake everything waiting on it.
    fn release_stream(&self, error: Option<&DriverError>) {
        let Ok(mut shared) = self.shared.lock() else {
            // A poisoned lock means the stream side already panicked, and
            // there is nothing left to wake.
            return;
        };

        if let Some(error) = error {
            let kind = match error.kind() {
                DriverErrorKind::Socket(io_error) => io_error.kind(),
                DriverErrorKind::Rdpeudp(_) => io::ErrorKind::InvalidData,
                DriverErrorKind::ConnectionClosed => io::ErrorKind::ConnectionAborted,
            };
            shared.error.get_or_insert(kind);
        }

        shared.close();
    }

    async fn run_to_completion(&mut self) -> Result<(), DriverError> {
        // Send any initial transmits (the SYN packet for client-side connections)
        self.drain_transmits().await?;

        loop {
            let timeout = self
                .conn
                .poll_timeout()
                .map(|deadline| tokio::time::Instant::from_std(self.clock.deadline(deadline)));

            // Reading a datagram may append to `read_buf`, so stop reading while
            // it is over its mark and wait for the consumer instead.
            let has_room = self.read_buf_has_room();

            // Not `biased`: branches 1-3 can all be genuinely ready at once
            // (incoming data, a queued TLS write, and an expired timer), and
            // fixed-order priority would let a continuously-readable socket
            // (steady traffic, or an adversarial ACK-only flood that never
            // fills read_buf) starve the write and timer branches forever.
            // Branch 0 and 1 are mutually exclusive via their `has_room`
            // guards, so fairness between them is moot either way.
            tokio::select! {
                // Branch 0: the consumer caught up, so go round again and read.
                _ = ReadBufDrained::new(&self.shared), if !has_room => {}

                // Branch 1: Incoming UDP datagram (highest priority)
                result = self.socket.recv(&mut self.recv_buf), if has_room => {
                    let n = result.map_err(|error| DriverError::socket("receive datagram", error))?;
                    let now = self.clock.now();

                    // handle_datagram takes &mut [u8] for in-place prefix byte swap
                    match self.conn.handle_datagram(&mut self.recv_buf[..n], now) {
                        Ok(()) => {}
                        // One datagram the state machine cannot make sense of
                        // is not grounds for tearing down the connection. UDP
                        // delivers whatever arrives at the socket, including
                        // corruption and anything an off-path sender puts
                        // there, and a peer whose retransmitted handshake
                        // datagram lands late produces this too. Drop it and
                        // carry on; a genuinely dead connection still closes
                        // on the idle timeout.
                        Err(error) if is_droppable(&error) => {
                            tracing::debug!(%error, "dropping an unusable datagram");
                        }
                        Err(error) => return Err(DriverError::rdpeudp("handle datagram", error)),
                    }

                    self.drain_transmits().await?;
                    self.drain_events();
                }

                // Branch 2: TLS layer has data to send. Guarded off while a
                // write is already pending backpressure: pulling more out of
                // `write_buf` on top of it would let newer bytes reach
                // `send()` before the older ones still waiting their turn,
                // corrupting the stream's order.
                _ = WriteDataReady::new(&self.shared), if self.pending_write.is_none() => {
                    let (data, stream_closed, flush_waker, room_waker) = {
                        let mut shared = self.shared.lock()
                            .map_err(|_| DriverError::connection_closed("lock shared state"))?;
                        let closed = shared.closed && shared.write_buf.is_empty();
                        let buf = shared.write_buf.split().freeze().to_vec();
                        let flush = shared.flush_waker.take();
                        let room = shared.write_room_waker.take();
                        (buf, closed, flush, room)
                    };
                    // Wake any pending flush (write_buf has been drained)
                    if let Some(waker) = flush_waker {
                        waker.wake();
                    }
                    // Wake a writer that was blocked on write_buf being full
                    if let Some(waker) = room_waker {
                        waker.wake();
                    }
                    if stream_closed {
                        self.conn.close();
                        self.drain_events();
                        return Ok(());
                    }
                    if !data.is_empty() && self.queue_write(data)? {
                        self.drain_transmits().await?;
                    }
                }

                // Branch 3: Timer expiry
                _ = optional_sleep(timeout) => {
                    let now = self.clock.now();
                    self.conn.handle_timeout(now);
                    self.drain_transmits().await?;
                    self.drain_events();
                }
            }

            // A write that hit backpressure earlier may have room now: an
            // incoming ACK (branch 1) or a retransmit (branch 3) can both
            // have just moved entries out of the send buffer via
            // `drain_transmits`. Self-guards on the connection already
            // being closed, so this is safe to call unconditionally rather
            // than threading it through every branch that might free room.
            self.flush_pending_write().await?;

            if self.conn.is_closed() {
                // Propagate close to the stream side
                let mut shared = self
                    .shared
                    .lock()
                    .map_err(|_| DriverError::connection_closed("lock shared state"))?;
                shared.close();
                return Ok(());
            }
        }
    }

    /// Whether `read_buf` is under its high-water mark.
    ///
    /// A poisoned lock means the stream side is gone, in which case there is
    /// no reason to keep throttling; the loop will notice and exit.
    fn read_buf_has_room(&self) -> bool {
        self.shared
            .lock()
            .map_or(true, |shared| shared.read_buf.len() < READ_BUF_HIGH_WATER)
    }

    /// Send all pending transmits to the UDP socket.
    async fn drain_transmits(&mut self) -> Result<(), DriverError> {
        let now = self.clock.now();
        while let Some(transmit) = self.conn.poll_transmit(now) {
            // `RdpeudpConnection` never hands back a V1Datagram once
            // established (only prefix-framed V2 packets), so there is
            // nothing to check on the data path.
            let bytes = if self.conn.is_established() {
                transmit.contents
            } else {
                pad_handshake_datagram(transmit.contents)
            };

            self.socket
                .send(&bytes)
                .await
                .map_err(|error| DriverError::socket("send datagram", error))?;
        }
        Ok(())
    }

    /// Hand `data` to the connection's send queue, or hold it in
    /// `pending_write` to retry if the queue is at its backpressure limit.
    ///
    /// `SendBufferFull` is not fatal: on a slow or lossy link, sustained
    /// writes can legitimately fill the bounded send queue while the
    /// connection is otherwise healthy, and the error kind's own
    /// documentation says the caller should retry rather than give up.
    /// Every other rejection stays fatal, unchanged.
    ///
    /// Returns whether `data` was actually queued, so the caller knows
    /// whether a `drain_transmits` is worth running.
    fn queue_write(&mut self, data: Vec<u8>) -> Result<bool, DriverError> {
        match self.conn.send(data) {
            Ok(()) => Ok(true),
            Err(SendError { error, data }) if matches!(error.kind(), RdpeudpErrorKind::SendBufferFull) => {
                self.pending_write = Some(data);
                Ok(false)
            }
            Err(SendError { error, .. }) => Err(DriverError::rdpeudp("queue outbound data", error)),
        }
    }

    /// Retry a write that hit backpressure last time, now that something
    /// may have drained the send buffer. A no-op if nothing is pending, or
    /// if the connection has already closed (retrying into a closed
    /// connection would surface as a fatal error and turn what should be a
    /// clean shutdown into one).
    async fn flush_pending_write(&mut self) -> Result<(), DriverError> {
        if self.conn.is_closed() {
            return Ok(());
        }

        let Some(data) = self.pending_write.take() else {
            return Ok(());
        };

        if self.queue_write(data)? {
            self.drain_transmits().await?;
        }

        Ok(())
    }

    /// Process all pending events from the connection state machine.
    fn drain_events(&mut self) {
        while let Some(event) = self.conn.poll_event() {
            match event {
                Event::Connected => {
                    if !self.connected_signaled {
                        self.connected_signaled = true;
                        self.connected_notify.notify_one();
                    }
                }
                Event::DataReceived(data) => {
                    if let Ok(mut shared) = self.shared.lock() {
                        shared.read_buf.extend_from_slice(&data);
                        if let Some(waker) = shared.read_waker.take() {
                            waker.wake();
                        }
                    }
                }
                Event::ConnectionClosed => {
                    if let Ok(mut shared) = self.shared.lock() {
                        shared.close();
                    }
                }
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════
// Helper futures
// ════════════════════════════════════════════════════════════════════

/// Future that resolves when `SharedIo::write_buf` has data OR when
/// the stream is shut down.
///
/// Cancel-safe: only registers a waker, no side effects on drop.
/// Resolves once `read_buf` has fallen back under its high-water mark.
struct ReadBufDrained {
    shared: Arc<Mutex<SharedIo>>,
}

impl ReadBufDrained {
    fn new(shared: &Arc<Mutex<SharedIo>>) -> Self {
        Self {
            shared: Arc::clone(shared),
        }
    }
}

impl Future for ReadBufDrained {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let Ok(mut shared) = self.shared.lock() else {
            return Poll::Ready(());
        };

        if shared.read_buf.len() < READ_BUF_HIGH_WATER || shared.closed {
            return Poll::Ready(());
        }

        shared.read_drained_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

struct WriteDataReady {
    shared: Arc<Mutex<SharedIo>>,
}

impl WriteDataReady {
    fn new(shared: &Arc<Mutex<SharedIo>>) -> Self {
        Self {
            shared: Arc::clone(shared),
        }
    }
}

impl Future for WriteDataReady {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let Ok(mut shared) = self.shared.lock() else {
            return Poll::Ready(());
        };

        if !shared.write_buf.is_empty() || shared.closed {
            return Poll::Ready(());
        }

        shared.write_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Sleep until the given deadline, or pend forever if `None`.
async fn optional_sleep(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => core::future::pending().await,
    }
}

/// Zero-pad a SYN or SYN+ACK datagram to the smaller of its own advertised
/// upstream and downstream MTU.
///
/// [MS-RDPEUDP] 3.1.5.1.1: "This datagram MUST be zero-padded to increase
/// the size of this datagram to uUpStreamMtu or uDownStreamMtu, whichever is
/// smaller", inherited by the SYN+ACK via 3.1.5.1.3 ("A SYN+ACK datagram
/// consists of a SYN packet, generated as specified in section 3.1.5.1.1").
/// `V1Datagram::encode` deliberately does not do this itself: it has no
/// socket to size against, so the responsibility falls to whoever hands the
/// bytes to one.
///
/// Every other handshake datagram (the plain final ACK) is returned
/// unchanged; §3.1.5.1.2 carries no padding requirement.
fn pad_handshake_datagram(mut bytes: Vec<u8>) -> Vec<u8> {
    let Ok(datagram) = ironrdp_core::decode::<V1Datagram>(&bytes) else {
        return bytes;
    };
    if !datagram.header.flags.contains(V1Flags::SYN) {
        return bytes;
    }
    let Some(syn_data) = datagram.syn_data.as_ref() else {
        return bytes;
    };

    let target = usize::from(syn_data.upstream_mtu.min(syn_data.downstream_mtu));
    if bytes.len() < target {
        bytes.resize(target, 0);
    }
    bytes
}

/// Whether a datagram that the state machine rejected can simply be dropped.
///
/// Anything the peer could put on the wire, deliberately or by accident, is
/// droppable: a malformed packet, a packet that makes no sense in the current
/// state. What is not droppable is the state machine reporting that this end
/// is finished, which is a real reason to stop the loop.
fn is_droppable(error: &RdpeudpError) -> bool {
    matches!(
        error.kind(),
        RdpeudpErrorKind::Decode(_)
            | RdpeudpErrorKind::Prefix(_)
            | RdpeudpErrorKind::InvalidPacket { .. }
            | RdpeudpErrorKind::InvalidState
    )
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use ironrdp_rdpeudp::ConnectionConfig;

    /// `connect` requires the cookie hash a version 3 SYN carries. These tests
    /// never reach a real multitransport request, so any value will do.
    fn test_connection_config() -> ConnectionConfig {
        ConnectionConfig {
            cookie_hash: Some([0x5A; 32]),
            ..ConnectionConfig::default()
        }
    }

    /// Build a client `RdpeudpConnection` past the handshake into
    /// `Established`, entirely in memory (no sockets), for tests that only
    /// care about data-phase behavior. Mirrors
    /// `ironrdp-testsuite-core/tests/rdpeudp/connection.rs`'s `establish_pair`;
    /// duplicated rather than shared because integration test binaries
    /// aren't importable from another crate's inline unit tests. If the
    /// handshake sequence changes there, update it here too.
    fn established_client_connection() -> RdpeudpConnection {
        let t = ironrdp_rdpeudp::MonotonicInstant::from_millis(0);

        let mut client = RdpeudpConnection::connect(test_connection_config(), t).expect("connect");
        let syn = client.poll_transmit(t).expect("SYN");

        let syn_dg: V1Datagram = ironrdp_core::decode(&syn.contents).expect("decode SYN");
        let mut server = RdpeudpConnection::accept(test_connection_config(), &syn_dg, t).expect("accept");
        let syn_ack = server.poll_transmit(t).expect("SYN+ACK");

        let mut syn_ack_bytes = syn_ack.contents;
        let handshake_rtt = t.saturating_add(Duration::from_millis(50));
        client
            .handle_datagram(&mut syn_ack_bytes, handshake_rtt)
            .expect("handle SYN+ACK");
        while client.poll_event().is_some() {}

        assert!(client.is_established(), "test setup: client should be established");
        client
    }

    /// Bind two UDP sockets on localhost and connect them to each other, for
    /// tests that need a `Driver`'s real socket send path to succeed without
    /// caring what (if anything) is on the other end.
    async fn connected_socket_pair() -> (UdpSocket, UdpSocket) {
        let a = UdpSocket::bind("127.0.0.1:0").await.expect("bind a");
        let b = UdpSocket::bind("127.0.0.1:0").await.expect("bind b");
        a.connect(b.local_addr().expect("addr b"))
            .await
            .expect("connect a to b");
        b.connect(a.local_addr().expect("addr a"))
            .await
            .expect("connect b to a");
        (a, b)
    }

    use ironrdp_rdpeudp::RdpeudpErrorExt as _;

    use super::*;

    #[tokio::test]
    async fn driver_sends_initial_syn() {
        // Bind two UDP sockets on localhost to simulate client/server
        let client_sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind client");
        let server_sock = UdpSocket::bind("127.0.0.1:0").await.expect("bind server");

        let server_addr = server_sock.local_addr().expect("server addr");
        client_sock.connect(server_addr).await.expect("connect");

        let conn = RdpeudpConnection::connect(test_connection_config(), Clock::new().now()).expect("connect");

        let shared = Arc::new(Mutex::new(SharedIo::new()));
        let notify = Arc::new(Notify::new());
        let driver = Driver::new(client_sock, conn, shared, notify);

        // Run the driver briefly: it should send the SYN packet
        let handle = tokio::spawn(driver.run());

        // Receive the SYN on the server side
        let mut buf = [0u8; 1500];
        let recv_result = tokio::time::timeout(Duration::from_secs(2), server_sock.recv(&mut buf)).await;

        assert!(recv_result.is_ok(), "should receive SYN packet");
        let n = recv_result.expect("timeout").expect("recv");
        assert_eq!(
            n,
            usize::from(ironrdp_rdpeudp::pdu::MTU_MAX),
            "the SYN should be zero-padded to the negotiated MTU, not sent at its natural encoded size"
        );

        // Abort the driver (we only needed to test the initial SYN)
        handle.abort();
    }

    /// The driver stops reading once `read_buf` is full, and starts again once
    /// the consumer has taken bytes out.
    ///
    /// The resume half is the part that is easy to leave out: without a wake
    /// from `poll_read` the driver sits throttled until some unrelated timer
    /// happens to fire, which on an idle connection is the keep-alive, eight
    /// seconds away.
    #[tokio::test]
    async fn a_full_read_buffer_throttles_the_driver_until_it_drains() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let conn = RdpeudpConnection::connect(test_connection_config(), Clock::new().now()).expect("connect");

        let shared = Arc::new(Mutex::new(SharedIo::new()));
        let driver = Driver::new(socket, conn, Arc::clone(&shared), Arc::new(Notify::new()));

        assert!(driver.read_buf_has_room(), "an empty buffer has room");

        shared
            .lock()
            .expect("lock")
            .read_buf
            .extend_from_slice(&vec![0u8; READ_BUF_HIGH_WATER]);

        assert!(!driver.read_buf_has_room(), "a full buffer does not");

        // Park on the resume future, then let a reader take the bytes out.
        let waiter = tokio::spawn({
            let shared = Arc::clone(&shared);
            async move { ReadBufDrained::new(&shared).await }
        });

        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "it should still be waiting");

        let mut stream = crate::stream::RdpeudpStream::new(Arc::clone(&shared));
        let mut buf = vec![0u8; READ_BUF_HIGH_WATER];
        tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
            .await
            .expect("read");

        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("the driver was never told the buffer drained")
            .expect("join");

        assert!(driver.read_buf_has_room());
    }

    /// Anything the peer can put on the wire is droppable; our own state
    /// machine telling us this end is finished is not.
    #[test]
    fn droppable_errors_are_the_ones_the_peer_controls() {
        assert!(is_droppable(&RdpeudpError::invalid_packet("test", "malformed")));
        assert!(!is_droppable(&RdpeudpError::connection_closed("test")));
    }

    /// A driver that exits on an error has to release the stream side. A
    /// reader parked in `poll_read` left its waker in the shared state and
    /// nothing else is going to poll it.
    #[tokio::test]
    async fn an_error_exit_wakes_a_parked_reader() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let conn = RdpeudpConnection::connect(test_connection_config(), Clock::new().now()).expect("connect");

        let shared = Arc::new(Mutex::new(SharedIo::new()));
        let woken = Arc::new(core::sync::atomic::AtomicBool::new(false));

        {
            let mut guard = shared.lock().expect("lock");
            guard.read_waker = Some(futures_waker(Arc::clone(&woken)));
        }

        let driver = Driver::new(socket, conn, Arc::clone(&shared), Arc::new(Notify::new()));
        driver.release_stream(Some(&DriverError::connection_closed("test")));

        assert!(
            woken.load(core::sync::atomic::Ordering::SeqCst),
            "the reader was left parked"
        );

        let guard = shared.lock().expect("lock");
        assert!(guard.closed);
        assert_eq!(guard.error, Some(io::ErrorKind::ConnectionAborted));
    }

    /// Build a waker that records having been woken.
    fn futures_waker(flag: Arc<core::sync::atomic::AtomicBool>) -> core::task::Waker {
        struct Flag(Arc<core::sync::atomic::AtomicBool>);

        impl std::task::Wake for Flag {
            fn wake(self: Arc<Self>) {
                self.0.store(true, core::sync::atomic::Ordering::SeqCst);
            }
        }

        core::task::Waker::from(Arc::new(Flag(flag)))
    }

    fn test_syn(upstream_mtu: u16, downstream_mtu: u16) -> V1Datagram {
        V1Datagram {
            header: ironrdp_rdpeudp::pdu::FecHeader {
                sn_source_ack: 0xFFFF_FFFF,
                receive_window_size: 64,
                flags: V1Flags::SYN,
            },
            ack_vector: None,
            ack_of_acks: None,
            syn_data: Some(ironrdp_rdpeudp::pdu::SynDataPayload {
                initial_sequence_number: 1,
                upstream_mtu,
                downstream_mtu,
            }),
            correlation_id: None,
            syn_data_ex: None,
        }
    }

    #[test]
    fn pads_a_syn_to_the_smaller_advertised_mtu() {
        let datagram = test_syn(1200, 1150);
        let encoded = ironrdp_core::encode_vec(&datagram).expect("encode");
        let natural_len = encoded.len();

        let padded = pad_handshake_datagram(encoded);

        assert_eq!(padded.len(), 1150, "should pad to the smaller of the two MTUs");
        assert!(
            padded[natural_len..].iter().all(|&b| b == 0),
            "padding must be zero bytes"
        );
        // The bytes before the padding are untouched.
        let redecoded: V1Datagram = ironrdp_core::decode(&padded).expect("decode padded");
        assert_eq!(redecoded.syn_data, datagram.syn_data);
    }

    #[test]
    fn pads_a_syn_ack_the_same_way() {
        let mut datagram = test_syn(1232, 1232);
        datagram.header.flags |= V1Flags::ACK;
        datagram.header.sn_source_ack = 42;
        let encoded = ironrdp_core::encode_vec(&datagram).expect("encode");

        let padded = pad_handshake_datagram(encoded);

        assert_eq!(padded.len(), 1232);
    }

    /// The plain final ACK carries no SYN flag, and MS-RDPEUDP 3.1.5.1.2
    /// gives it no padding requirement.
    #[test]
    fn leaves_the_final_ack_unpadded() {
        let datagram = V1Datagram {
            header: ironrdp_rdpeudp::pdu::FecHeader {
                sn_source_ack: 7,
                receive_window_size: 64,
                flags: V1Flags::ACK,
            },
            ack_vector: Some(ironrdp_rdpeudp::pdu::V1AckVectorHeader {
                elements: vec![ironrdp_rdpeudp::pdu::V1AckVectorElement {
                    state: ironrdp_rdpeudp::pdu::VectorElementState::DatagramReceived,
                    length: 1,
                }],
            }),
            ack_of_acks: None,
            syn_data: None,
            correlation_id: None,
            syn_data_ex: None,
        };
        let encoded = ironrdp_core::encode_vec(&datagram).expect("encode");
        let natural_len = encoded.len();

        let padded = pad_handshake_datagram(encoded);

        assert_eq!(padded.len(), natural_len, "a plain ACK must not be padded");
    }

    /// Bytes that don't decode as a V1Datagram at all (e.g. a prefix-framed
    /// V2 data packet, which the caller never actually passes here, but the
    /// function must still be safe against) pass through unchanged.
    #[test]
    fn leaves_non_v1_datagram_bytes_unchanged() {
        let bytes = vec![0xFFu8; 12];
        let padded = pad_handshake_datagram(bytes.clone());
        assert_eq!(padded, bytes);
    }

    /// This is the actual defect: `RdpeudpConnection::send` returning
    /// `SendBufferFull` used to propagate as a fatal `DriverError` via `?`,
    /// tearing down an otherwise-healthy connection over transient
    /// backpressure. `queue_write` must hold the data and report it as
    /// merely not-yet-queued, never as an error, for that one kind.
    #[tokio::test]
    async fn queue_write_holds_data_instead_of_erroring_when_the_send_buffer_is_full() {
        let conn = established_client_connection();
        let (socket, _peer) = connected_socket_pair().await;
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        let notify = Arc::new(Notify::new());
        let mut driver = Driver::new(socket, conn, shared, notify);

        // test_connection_config's log_window_size is 6 (64 slots); the
        // bound is 8x that, matching ironrdp-testsuite-core's own
        // send-buffer-full test. queue_write never calls drain_transmits,
        // so nothing drains the buffer as it fills.
        for _ in 0..512 {
            assert!(
                driver
                    .queue_write(vec![0xAAu8; 4])
                    .expect("queue_write should not error while filling"),
                "every write under the bound should be queued"
            );
        }

        let queued = driver
            .queue_write(vec![0xBBu8; 4])
            .expect("SendBufferFull must not surface as a DriverError");
        assert!(!queued, "the write past the bound should be held, not queued");
        assert_eq!(
            driver.pending_write.as_deref(),
            Some([0xBBu8; 4].as_slice()),
            "the exact rejected payload should be preserved, not dropped"
        );
    }

    /// `flush_pending_write` is a no-op, not an error, when there is nothing
    /// held.
    #[tokio::test]
    async fn flush_pending_write_is_a_no_op_with_nothing_pending() {
        let conn = established_client_connection();
        let (socket, _peer) = connected_socket_pair().await;
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        let notify = Arc::new(Notify::new());
        let mut driver = Driver::new(socket, conn, shared, notify);

        driver.flush_pending_write().await.expect("no-op should not error");
        assert!(driver.pending_write.is_none());
    }

    /// Once `drain_transmits` moves entries out of the send buffer, a held
    /// write is retried and actually goes out, closing the loop on the
    /// backpressure fix rather than leaving data stuck forever.
    #[tokio::test]
    async fn flush_pending_write_delivers_once_the_send_buffer_has_room() {
        let conn = established_client_connection();
        let (socket, _peer) = connected_socket_pair().await;
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        let notify = Arc::new(Notify::new());
        let mut driver = Driver::new(socket, conn, shared, notify);

        for _ in 0..512 {
            driver.queue_write(vec![0xAAu8; 4]).expect("queue_write");
        }
        let queued = driver.queue_write(vec![0xCCu8; 4]).expect("queue_write");
        assert!(!queued, "test setup: the buffer should be full");
        assert!(driver.pending_write.is_some());

        // Only draining what's already queued frees room; nothing else in
        // this test touches the send buffer.
        driver.drain_transmits().await.expect("drain_transmits");

        driver.flush_pending_write().await.expect("flush_pending_write");
        assert!(
            driver.pending_write.is_none(),
            "the held write should have been queued and sent once room opened"
        );
    }

    /// `flush_pending_write` must not attempt the retry once the connection
    /// has already closed: `send()` on a closed connection is a fatal error,
    /// which would turn a clean shutdown into a spurious one.
    #[tokio::test]
    async fn flush_pending_write_does_not_error_on_an_already_closed_connection() {
        let mut conn = established_client_connection();
        conn.close();
        assert!(conn.is_closed(), "test setup: connection should be closed");

        let (socket, _peer) = connected_socket_pair().await;
        let shared = Arc::new(Mutex::new(SharedIo::new()));
        let notify = Arc::new(Notify::new());
        let mut driver = Driver::new(socket, conn, shared, notify);
        driver.pending_write = Some(vec![0xDDu8; 4]);

        driver
            .flush_pending_write()
            .await
            .expect("a closed connection must not turn the flush into an error");
    }

    /// This is the gap the isolated `queue_write`/`flush_pending_write` tests
    /// above cannot reach on their own: does the real `select!` loop, driving
    /// a real connection against a peer that never acks, actually stop
    /// `write_buf` from growing without bound the way those tests assume?
    ///
    /// The peer socket is bound and connected but never read from, so
    /// nothing ever acknowledges anything: the congestion window never grows
    /// past its initial allowance, `send_buffer` fills, `pending_write`
    /// engages and never clears, and branch 2 stops draining `write_buf` for
    /// the rest of the test. It is deliberately not dropped: a closed peer
    /// socket can turn a subsequent send into a real `ECONNREFUSED`, which
    /// would kill the driver with a fatal error instead of sustaining the
    /// congestion this test needs.
    #[tokio::test(flavor = "multi_thread")]
    async fn sustained_writes_against_an_unresponsive_peer_bound_write_buf() {
        let conn = established_client_connection();
        let (socket, unresponsive_peer) = connected_socket_pair().await;

        let shared = Arc::new(Mutex::new(SharedIo::new()));
        let notify = Arc::new(Notify::new());
        let driver = Driver::new(socket, conn, Arc::clone(&shared), notify);
        let driver_handle = tokio::spawn(driver.run());

        let mut stream = crate::stream::RdpeudpStream::new(Arc::clone(&shared));

        // Keep writing well past WRITE_BUF_HIGH_WATER. If the bound holds,
        // this never completes inside the timeout.
        let outcome = tokio::time::timeout(Duration::from_secs(5), async {
            let chunk = vec![0xABu8; 4096];
            // Comfortably more than WRITE_BUF_HIGH_WATER / chunk.len() (256):
            // if the bound holds, this never gets close to finishing before
            // the timeout above fires, since a blocked write just sits
            // pending. If the bound is broken, this actually would run to
            // completion instead.
            for _ in 0..100_000 {
                tokio::io::AsyncWriteExt::write_all(&mut stream, &chunk)
                    .await
                    .expect("write_all");
            }
        })
        .await;

        assert!(
            outcome.is_err(),
            "writes should have blocked at the high-water mark well before completing 100,000 chunks"
        );

        let buf_len = shared.lock().expect("lock").write_buf.len();
        assert!(
            buf_len <= crate::stream::WRITE_BUF_HIGH_WATER,
            "write_buf grew to {buf_len} bytes, past its {}-byte bound",
            crate::stream::WRITE_BUF_HIGH_WATER
        );

        driver_handle.abort();
        drop(unresponsive_peer);
    }
}
