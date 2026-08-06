//! Tokio I/O adapter for the reliable RDP-UDP transport.
//!
//! [`UdpTransportStream`] is a byte stream over reliable RDP-UDP. It does not
//! implement RDPEMT and can therefore be passed directly to a TLS layer.

use core::fmt;
use core::net::SocketAddr;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll};
use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use ironrdp_rdpudp::{
    Config, Datagram, Error as EngineError, MAX_DATAGRAM_SIZE, MIN_MTU, ReliableUdp, State, Timestamp,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::warn;

const CHANNEL_CAPACITY: usize = 32;

/// Errors returned while creating, operating, or closing a UDP transport stream.
#[derive(Debug)]
#[non_exhaustive]
pub enum UdpTransportError {
    /// The listener connection limit was zero.
    InvalidConnectionLimit,
    /// The reliable RDP-UDP engine rejected a configuration or packet.
    Engine(ironrdp_rdpudp::Error),
    /// A socket operation failed.
    Io(io::Error),
    /// The background transport task stopped unexpectedly.
    DriverJoin(tokio::task::JoinError),
}

impl fmt::Display for UdpTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConnectionLimit => f.write_str("UDP connection limit must be non-zero"),
            Self::Engine(error) => write!(f, "RDP-UDP engine error: {error:?}"),
            Self::Io(error) => write!(f, "UDP socket error: {error}"),
            Self::DriverJoin(error) => write!(f, "RDP-UDP driver task failed: {error}"),
        }
    }
}

impl core::error::Error for UdpTransportError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::InvalidConnectionLimit | Self::Engine(_) => None,
            Self::Io(error) => Some(error),
            Self::DriverJoin(error) => Some(error),
        }
    }
}

impl From<ironrdp_rdpudp::Error> for UdpTransportError {
    fn from(error: ironrdp_rdpudp::Error) -> Self {
        Self::Engine(error)
    }
}

impl From<io::Error> for UdpTransportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A reliable RDP-UDP connection accepted by [`UdpTransportListener`].
pub struct AcceptedUdpTransport {
    /// UDP endpoint of the connected peer.
    pub peer: SocketAddr,
    /// Ordered reliable byte stream for the peer.
    pub stream: UdpTransportStream,
}

/// A bounded shared-socket listener for reliable RDP-UDP connections.
///
/// Retain this listener while using accepted streams: its background task owns
/// datagram demultiplexing for every peer sharing its UDP socket.
pub struct UdpTransportListener {
    socket: Arc<UdpSocket>,
    accepted_rx: mpsc::Receiver<AcceptedUdpTransport>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    driver: Option<JoinHandle<Result<(), UdpTransportError>>>,
}

impl UdpTransportListener {
    /// Binds `local_addr` and accepts at most `max_connections` peers concurrently.
    pub async fn bind(
        local_addr: SocketAddr,
        config: Config,
        max_connections: usize,
    ) -> Result<Self, UdpTransportError> {
        if max_connections == 0 {
            return Err(UdpTransportError::InvalidConnectionLimit);
        }

        config.validate()?;
        let socket = Arc::new(UdpSocket::bind(local_addr).await?);
        let (accepted_tx, accepted_rx) = mpsc::channel(max_connections);
        let (opened_tx, opened_rx) = mpsc::channel(max_connections);
        let (closed_tx, closed_rx) = mpsc::channel(max_connections);
        let (cleanup_tx, cleanup_rx) = mpsc::channel(max_connections);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let driver = tokio::spawn(run_listener(
            Arc::clone(&socket),
            config,
            max_connections,
            accepted_tx,
            opened_rx,
            closed_rx,
            cleanup_rx,
            opened_tx,
            closed_tx,
            cleanup_tx,
            shutdown_rx,
        ));

        Ok(Self {
            socket,
            accepted_rx,
            shutdown_tx: Some(shutdown_tx),
            driver: Some(driver),
        })
    }

    /// Returns the local address of the shared UDP socket.
    pub fn local_addr(&self) -> Result<SocketAddr, UdpTransportError> {
        Ok(self.socket.local_addr()?)
    }

    /// Waits for a client that completed the reliable RDP-UDP handshake.
    pub async fn accept(&mut self) -> Option<AcceptedUdpTransport> {
        self.accepted_rx.recv().await
    }

    /// Stops datagram demultiplexing and waits for the listener task to finish.
    pub async fn close(mut self) -> Result<(), UdpTransportError> {
        self.shutdown_tx.take();
        let Some(driver) = self.driver.take() else {
            return Ok(());
        };

        driver.await.map_err(UdpTransportError::DriverJoin)?
    }
}

impl Drop for UdpTransportListener {
    fn drop(&mut self) {
        if let Some(driver) = &self.driver {
            driver.abort();
        }
    }
}

/// Connects an ephemeral UDP socket to `peer` and starts a reliable RDP-UDP byte stream.
///
/// The returned stream preserves byte ordering, but internally splits writes
/// into packets no larger than the configured RDP-UDP MTU. The initial
/// RDP-UDP SYN is sent before this function returns; TLS and RDPEMT setup
/// remain the responsibility of the caller.
pub async fn connect(peer: SocketAddr, config: Config) -> Result<UdpTransportStream, UdpTransportError> {
    let max_write_size = Arc::new(AtomicUsize::new(MIN_MTU));
    let mut transport = ReliableUdp::new(config)?;
    let local_addr = if peer.is_ipv4() {
        SocketAddr::from(([0, 0, 0, 0], 0))
    } else {
        SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 0], 0))
    };
    let socket = UdpSocket::bind(local_addr).await?;
    socket.connect(peer).await?;

    let started_at = Instant::now();
    let initial_datagram = transport.start(timestamp(started_at))?;
    send_datagrams(&socket, core::iter::once(initial_datagram)).await?;

    let (inbound_tx, inbound_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (outbound_tx, outbound_rx) = mpsc::channel(CHANNEL_CAPACITY);
    let (read_progress_tx, read_progress_rx) = mpsc::unbounded_channel();
    let receive_buffer_size = MAX_DATAGRAM_SIZE;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let driver = tokio::spawn(run_driver(
        socket,
        transport,
        started_at,
        inbound_tx,
        outbound_rx,
        read_progress_rx,
        shutdown_rx,
        receive_buffer_size,
        Arc::clone(&max_write_size),
    ));

    Ok(UdpTransportStream {
        inbound_rx,
        inbound_buffer: Vec::new(),
        inbound_offset: 0,
        read_progress_tx,
        outbound_tx: Some(outbound_tx),
        pending_write: None,
        shutdown_tx: Some(shutdown_tx),
        driver: Some(driver),
        max_write_size,
        peer_cleanup: None,
    })
}

/// An ordered byte stream carried by a reliable RDP-UDP connection.
///
/// Dropping the stream aborts its background driver. Call [`Self::close`] or
/// use [`AsyncWrite::poll_shutdown`] to stop the driver and observe its result.
pub struct UdpTransportStream {
    inbound_rx: mpsc::Receiver<Vec<u8>>,
    inbound_buffer: Vec<u8>,
    inbound_offset: usize,
    read_progress_tx: mpsc::UnboundedSender<()>,
    outbound_tx: Option<mpsc::Sender<Vec<u8>>>,
    pending_write: Option<PendingWrite>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    driver: Option<JoinHandle<Result<(), UdpTransportError>>>,
    max_write_size: Arc<AtomicUsize>,
    peer_cleanup: Option<PeerCleanup>,
}

struct PendingWrite {
    data: Vec<u8>,
    reserve: ReserveFuture,
}

type ReserveFuture =
    Pin<Box<dyn Future<Output = Result<mpsc::OwnedPermit<Vec<u8>>, mpsc::error::SendError<()>>> + Send>>;

struct PeerCleanup {
    peer: SocketAddr,
    tx: mpsc::Sender<SocketAddr>,
}

impl UdpTransportStream {
    /// Stops the background driver and waits for it to finish.
    pub async fn close(mut self) -> Result<(), UdpTransportError> {
        self.begin_graceful_shutdown();
        let Some(driver) = self.driver.take() else {
            self.finish_close();
            return Ok(());
        };

        let result = driver.await.map_err(UdpTransportError::DriverJoin)?;
        self.finish_close();
        result
    }

    fn begin_graceful_shutdown(&mut self) {
        self.outbound_tx.take();
        self.pending_write.take();
    }

    fn finish_close(&mut self) {
        self.shutdown_tx.take();
        self.inbound_rx.close();
        if let Some(PeerCleanup { peer, tx }) = self.peer_cleanup.take() {
            let _ = tx.try_send(peer);
        }
    }

    fn abort_channels(&mut self) {
        self.begin_graceful_shutdown();
        self.finish_close();
    }

    fn poll_driver_completion(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let Some(driver) = self.driver.as_mut() else {
            return Poll::Ready(Ok(()));
        };

        match Pin::new(driver).poll(cx) {
            Poll::Ready(Ok(Ok(()))) => {
                self.driver.take();
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Ok(Err(error))) => {
                self.driver.take();
                Poll::Ready(Err(io::Error::other(error)))
            }
            Poll::Ready(Err(error)) => {
                self.driver.take();
                Poll::Ready(Err(io::Error::other(UdpTransportError::DriverJoin(error))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for UdpTransportStream {
    fn drop(&mut self) {
        self.abort_channels();
        if let Some(driver) = &self.driver {
            driver.abort();
        }
    }
}

impl AsyncRead for UdpTransportStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        loop {
            if self.inbound_offset < self.inbound_buffer.len() {
                let available = &self.inbound_buffer[self.inbound_offset..];
                let read_len = available.len().min(buf.remaining());
                buf.put_slice(&available[..read_len]);
                self.inbound_offset += read_len;
                if self.inbound_offset == self.inbound_buffer.len() {
                    let _ = self.read_progress_tx.send(());
                }
                return Poll::Ready(Ok(()));
            }

            self.inbound_buffer.clear();
            self.inbound_offset = 0;
            match self.inbound_rx.poll_recv(cx) {
                Poll::Ready(Some(buffer)) if buffer.is_empty() => {
                    let _ = self.read_progress_tx.send(());
                }
                Poll::Ready(Some(buffer)) => self.inbound_buffer = buffer,
                Poll::Ready(None) => return self.poll_driver_completion(cx),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl AsyncWrite for UdpTransportStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        if self.pending_write.is_none() {
            let write_len = buf.len().min(self.max_write_size.load(Ordering::Acquire));
            let Some(sender) = self.outbound_tx.as_ref() else {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "RDP-UDP stream is closed",
                )));
            };
            self.pending_write = Some(PendingWrite {
                data: buf[..write_len].to_vec(),
                reserve: Box::pin(sender.clone().reserve_owned()),
            });
        }

        let reserve_poll = match self.pending_write.as_mut() {
            Some(pending_write) => pending_write.reserve.as_mut().poll(cx),
            None => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "RDP-UDP stream is closed",
                )));
            }
        };

        match reserve_poll {
            Poll::Ready(Ok(permit)) => {
                let Some(PendingWrite { data, .. }) = self.pending_write.take() else {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "RDP-UDP stream is closed",
                    )));
                };
                let write_len = data.len();
                permit.send(data);
                Poll::Ready(Ok(write_len))
            }
            Poll::Ready(Err(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "RDP-UDP driver has stopped",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.begin_graceful_shutdown();
        match self.poll_driver_completion(cx) {
            Poll::Ready(result) => {
                self.finish_close();
                Poll::Ready(result)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

struct Peer {
    packet_tx: mpsc::Sender<Vec<u8>>,
    stream: Option<PendingPeerStream>,
}

struct PendingPeerStream {
    inbound_rx: mpsc::Receiver<Vec<u8>>,
    read_progress_tx: mpsc::UnboundedSender<()>,
    outbound_tx: mpsc::Sender<Vec<u8>>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    driver: JoinHandle<Result<(), UdpTransportError>>,
    max_write_size: Arc<AtomicUsize>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the listener task owns each bounded channel and its socket separately"
)]
async fn run_listener(
    socket: Arc<UdpSocket>,
    config: Config,
    max_connections: usize,
    accepted_tx: mpsc::Sender<AcceptedUdpTransport>,
    mut opened_rx: mpsc::Receiver<SocketAddr>,
    mut closed_rx: mpsc::Receiver<SocketAddr>,
    mut cleanup_rx: mpsc::Receiver<SocketAddr>,
    opened_tx: mpsc::Sender<SocketAddr>,
    closed_tx: mpsc::Sender<SocketAddr>,
    cleanup_tx: mpsc::Sender<SocketAddr>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), UdpTransportError> {
    let mut peers = HashMap::with_capacity(max_connections);
    let mut receive_buffer = vec![0; MAX_DATAGRAM_SIZE];

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                abort_pending_peer_drivers(&mut peers);
                return Ok(());
            }
            Some(peer) = opened_rx.recv() => {
                let Some(entry) = peers.get_mut(&peer) else {
                    continue;
                };
                let Some(stream) = entry.stream.take() else {
                    continue;
                };
                let stream = UdpTransportStream {
                    inbound_rx: stream.inbound_rx,
                    inbound_buffer: Vec::new(),
                    inbound_offset: 0,
                    read_progress_tx: stream.read_progress_tx,
                    outbound_tx: Some(stream.outbound_tx),
                    pending_write: None,
                    shutdown_tx: Some(stream.shutdown_tx),
                    driver: Some(stream.driver),
                    max_write_size: stream.max_write_size,
                    peer_cleanup: Some(PeerCleanup {
                        peer,
                        tx: cleanup_tx.clone(),
                    }),
                };
                if accepted_tx.send(AcceptedUdpTransport { peer, stream }).await.is_err() {
                    abort_pending_peer_drivers(&mut peers);
                    return Ok(());
                }
            }
            Some(peer) = closed_rx.recv() => remove_peer(&mut peers, peer),
            Some(peer) = cleanup_rx.recv() => remove_peer(&mut peers, peer),
            received = socket.recv_from(&mut receive_buffer) => {
                let (received, peer) = match received {
                    Ok(received) => received,
                    Err(error) => {
                        warn!(%error, "RDP-UDP listener receive failed");
                        continue;
                    }
                };
                if let Some(entry) = peers.get(&peer) {
                    match entry.packet_tx.clone().try_reserve_owned() {
                        Ok(permit) => {
                            permit.send(receive_buffer[..received].to_vec());
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => {}
                        Err(mpsc::error::TrySendError::Closed(_)) => remove_peer(&mut peers, peer),
                    }
                    continue;
                }

                if peers.len() >= max_connections {
                    continue;
                }

                let started_at = Instant::now();
                let Ok((transport, syn_ack)) = ReliableUdp::accept(config, timestamp(started_at), &receive_buffer[..received]) else {
                    continue;
                };
                if let Err(error) = send_datagrams_to(socket.as_ref(), peer, core::iter::once(syn_ack)).await {
                    warn!(%peer, %error, "Failed to send RDP-UDP SYN+ACK");
                    continue;
                }

                let max_write_size = Arc::new(AtomicUsize::new(transport.maximum_source_payload_size()));
                let (packet_tx, packet_rx) = mpsc::channel(CHANNEL_CAPACITY);
                let (inbound_tx, inbound_rx) = mpsc::channel(CHANNEL_CAPACITY);
                let (outbound_tx, outbound_rx) = mpsc::channel(CHANNEL_CAPACITY);
                let (read_progress_tx, read_progress_rx) = mpsc::unbounded_channel();
                let (peer_shutdown_tx, peer_shutdown_rx) = tokio::sync::oneshot::channel();
                let driver = tokio::spawn(run_peer_driver(
                    Arc::clone(&socket),
                    peer,
                    transport,
                    started_at,
                    packet_rx,
                    inbound_tx,
                    outbound_rx,
                    read_progress_rx,
                    peer_shutdown_rx,
                    opened_tx.clone(),
                    closed_tx.clone(),
                    Arc::clone(&max_write_size),
                ));
                peers.insert(
                    peer,
                    Peer {
                        packet_tx,
                        stream: Some(PendingPeerStream {
                            inbound_rx,
                            read_progress_tx,
                            outbound_tx,
                            shutdown_tx: peer_shutdown_tx,
                            driver,
                            max_write_size,
                        }),
                    },
                );
            }
        }
    }
}

fn remove_peer(peers: &mut HashMap<SocketAddr, Peer>, peer: SocketAddr) {
    let Some(mut entry) = peers.remove(&peer) else {
        return;
    };
    if let Some(stream) = entry.stream.take() {
        stream.driver.abort();
    }
}

fn abort_pending_peer_drivers(peers: &mut HashMap<SocketAddr, Peer>) {
    for entry in peers.values_mut() {
        if let Some(stream) = entry.stream.as_mut() {
            stream.driver.abort();
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the peer task receives independently owned channels and shared socket state"
)]
async fn run_peer_driver(
    socket: Arc<UdpSocket>,
    peer: SocketAddr,
    transport: ReliableUdp,
    started_at: Instant,
    packet_rx: mpsc::Receiver<Vec<u8>>,
    inbound_tx: mpsc::Sender<Vec<u8>>,
    outbound_rx: mpsc::Receiver<Vec<u8>>,
    read_progress_rx: mpsc::UnboundedReceiver<()>,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    opened_tx: mpsc::Sender<SocketAddr>,
    closed_tx: mpsc::Sender<SocketAddr>,
    max_write_size: Arc<AtomicUsize>,
) -> Result<(), UdpTransportError> {
    let result = run_shared_peer_driver(
        socket,
        peer,
        transport,
        started_at,
        packet_rx,
        inbound_tx,
        outbound_rx,
        read_progress_rx,
        shutdown_rx,
        opened_tx,
        max_write_size,
    )
    .await;
    let _ = closed_tx.send(peer).await;
    result
}

#[expect(
    clippy::too_many_arguments,
    reason = "the peer task receives independently owned channels and shared socket state"
)]
async fn run_shared_peer_driver(
    socket: Arc<UdpSocket>,
    peer: SocketAddr,
    mut transport: ReliableUdp,
    started_at: Instant,
    mut packet_rx: mpsc::Receiver<Vec<u8>>,
    inbound_tx: mpsc::Sender<Vec<u8>>,
    mut outbound_rx: mpsc::Receiver<Vec<u8>>,
    mut read_progress_rx: mpsc::UnboundedReceiver<()>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    opened_tx: mpsc::Sender<SocketAddr>,
    max_write_size: Arc<AtomicUsize>,
) -> Result<(), UdpTransportError> {
    let mut pending_message = None;
    let mut opened = false;

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return Ok(()),
            packet = packet_rx.recv() => {
                let Some(packet) = packet else {
                    return Ok(());
                };
                let Some(datagrams) = handle_incoming_datagram(&mut transport, timestamp(started_at), &packet)? else {
                    continue;
                };
                send_datagrams_to(socket.as_ref(), peer, datagrams).await?;
                if !forward_received(&mut transport, &inbound_tx) {
                    return Ok(());
                }
            }
            Some(()) = read_progress_rx.recv() => {
                let mut consumed: usize = 1;
                while read_progress_rx.try_recv().is_ok() {
                    consumed = consumed.saturating_add(1);
                }
                transport.release_runtime_delivery(consumed)?;
                if !forward_received(&mut transport, &inbound_tx) {
                    return Ok(());
                }
                let acknowledgement = transport.acknowledge_receive_window(timestamp(started_at))?;
                send_datagrams_to(socket.as_ref(), peer, core::iter::once(acknowledgement)).await?;
            }
            message = outbound_rx.recv(), if pending_message.is_none() => {
                let Some(message) = message else {
                    return Ok(());
                };
                pending_message = Some(message);
            }
            () = wait_for_deadline(&transport, started_at) => {
                let datagrams = transport.handle_timeout(timestamp(started_at))?;
                send_datagrams_to(socket.as_ref(), peer, datagrams).await?;
            }
        }

        if !opened && matches!(transport.state(), State::Open { .. }) {
            update_max_write_size(&transport, &max_write_size);
            if opened_tx.send(peer).await.is_err() {
                return Ok(());
            }
            opened = true;
        }
        send_pending_message_to(socket.as_ref(), peer, &mut transport, started_at, &mut pending_message).await?;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the client driver receives independently owned socket and channel state"
)]
async fn run_driver(
    socket: UdpSocket,
    mut transport: ReliableUdp,
    started_at: Instant,
    inbound_tx: mpsc::Sender<Vec<u8>>,
    mut outbound_rx: mpsc::Receiver<Vec<u8>>,
    mut read_progress_rx: mpsc::UnboundedReceiver<()>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    receive_buffer_size: usize,
    max_write_size: Arc<AtomicUsize>,
) -> Result<(), UdpTransportError> {
    let mut receive_buffer = vec![0; receive_buffer_size];
    let mut pending_message = None;

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return Ok(()),
            received = socket.recv(&mut receive_buffer) => {
                let received = received?;
                let Some(datagrams) = handle_incoming_datagram(
                    &mut transport,
                    timestamp(started_at),
                    &receive_buffer[..received],
                )? else {
                    continue;
                };
                send_datagrams(&socket, datagrams).await?;
                if !forward_received(&mut transport, &inbound_tx) {
                    return Ok(());
                }
            }
            Some(()) = read_progress_rx.recv() => {
                let mut consumed: usize = 1;
                while read_progress_rx.try_recv().is_ok() {
                    consumed = consumed.saturating_add(1);
                }
                transport.release_runtime_delivery(consumed)?;
                if !forward_received(&mut transport, &inbound_tx) {
                    return Ok(());
                }
                let acknowledgement = transport.acknowledge_receive_window(timestamp(started_at))?;
                send_datagrams(&socket, core::iter::once(acknowledgement)).await?;
            }
            message = outbound_rx.recv(), if pending_message.is_none() => {
                let Some(message) = message else {
                    return Ok(());
                };
                pending_message = Some(message);
            }
            () = wait_for_deadline(&transport, started_at) => {
                let datagrams = transport.handle_timeout(timestamp(started_at))?;
                send_datagrams(&socket, datagrams).await?;
            }
        }

        update_max_write_size(&transport, &max_write_size);
        send_pending_message(&socket, &mut transport, started_at, &mut pending_message).await?;
    }
}

fn handle_incoming_datagram(
    transport: &mut ReliableUdp,
    now: Timestamp,
    datagram: &[u8],
) -> Result<Option<Vec<Datagram>>, UdpTransportError> {
    match transport.handle_datagram(now, datagram) {
        Ok(datagrams) => Ok(Some(datagrams)),
        Err(error) if transport.state() == State::Failed => Err(error.into()),
        Err(error) => {
            warn!(?error, "Ignoring invalid RDP-UDP datagram");
            Ok(None)
        }
    }
}

async fn send_pending_message(
    socket: &UdpSocket,
    transport: &mut ReliableUdp,
    started_at: Instant,
    pending_message: &mut Option<Vec<u8>>,
) -> Result<(), UdpTransportError> {
    let Some(message) = pending_message.as_ref() else {
        return Ok(());
    };

    match transport.state() {
        State::SynSent | State::SynReceived { .. } => return Ok(()),
        State::Open { .. } => {}
        State::Failed => return Err(EngineError::RetransmitLimitReached.into()),
        State::Idle => return Err(EngineError::InvalidState.into()),
    }

    match transport.send(timestamp(started_at), message.clone()) {
        Ok(datagram) => {
            pending_message.take();
            send_datagrams(socket, core::iter::once(datagram)).await
        }
        Err(EngineError::SendWindowFull) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn send_pending_message_to(
    socket: &UdpSocket,
    peer: SocketAddr,
    transport: &mut ReliableUdp,
    started_at: Instant,
    pending_message: &mut Option<Vec<u8>>,
) -> Result<(), UdpTransportError> {
    let Some(message) = pending_message.as_ref() else {
        return Ok(());
    };

    match transport.state() {
        State::SynSent | State::SynReceived { .. } => return Ok(()),
        State::Open { .. } => {}
        State::Failed => return Err(EngineError::RetransmitLimitReached.into()),
        State::Idle => return Err(EngineError::InvalidState.into()),
    }

    match transport.send(timestamp(started_at), message.clone()) {
        Ok(datagram) => {
            pending_message.take();
            send_datagrams_to(socket, peer, core::iter::once(datagram)).await
        }
        Err(EngineError::SendWindowFull) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn forward_received(transport: &mut ReliableUdp, inbound_tx: &mpsc::Sender<Vec<u8>>) -> bool {
    while transport.has_received() {
        let permit = match inbound_tx.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => return true,
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
        };
        let Some(message) = transport.take_received_for_runtime() else {
            return true;
        };
        permit.send(message);
    }

    true
}

async fn send_datagrams(
    socket: &UdpSocket,
    datagrams: impl IntoIterator<Item = Datagram>,
) -> Result<(), UdpTransportError> {
    for Datagram(datagram) in datagrams {
        let sent = socket.send(&datagram).await?;
        if sent != datagram.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "failed to send complete UDP datagram").into());
        }
    }

    Ok(())
}

async fn send_datagrams_to(
    socket: &UdpSocket,
    peer: SocketAddr,
    datagrams: impl IntoIterator<Item = Datagram>,
) -> Result<(), UdpTransportError> {
    for Datagram(datagram) in datagrams {
        let sent = socket.send_to(&datagram, peer).await?;
        if sent != datagram.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "failed to send complete UDP datagram").into());
        }
    }

    Ok(())
}

async fn wait_for_deadline(transport: &ReliableUdp, started_at: Instant) {
    if let Some(deadline) = transport.next_deadline() {
        tokio::time::sleep_until(started_at + deadline.elapsed()).await;
    } else {
        core::future::pending::<()>().await;
    }
}

fn timestamp(started_at: Instant) -> Timestamp {
    Timestamp::from_elapsed(started_at.elapsed())
}

fn update_max_write_size(transport: &ReliableUdp, max_write_size: &AtomicUsize) {
    if matches!(transport.state(), State::Open { .. }) {
        max_write_size.store(transport.maximum_source_payload_size(), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use core::error::Error;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::time::Duration;
    use std::sync::Arc;

    use ironrdp_rdpudp::{MIN_MTU, ProtocolVersion};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::sync::mpsc;

    use super::{
        CHANNEL_CAPACITY, Config, EngineError, ReliableUdp, State, Timestamp, UdpTransportError, UdpTransportListener,
        UdpTransportStream, connect, handle_incoming_datagram,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    fn config() -> Config {
        Config {
            initial_sequence_number: 1,
            max_version: ProtocolVersion::V2,
            receive_window_size: 32,
            mtu: 1200,
            max_retransmits: 3,
            max_reorder_buffer: 32,
            max_delivered_messages: 32,
        }
    }

    #[tokio::test]
    async fn listener_rejects_zero_connection_limit() -> TestResult {
        let local_addr = "127.0.0.1:0".parse()?;
        let error = UdpTransportListener::bind(local_addr, config(), 0)
            .await
            .err()
            .ok_or_else(|| std::io::Error::other("listener accepted a zero connection limit"))?;
        assert!(matches!(error, UdpTransportError::InvalidConnectionLimit));
        Ok(())
    }

    #[tokio::test]
    async fn listener_accepts_and_relays_bidirectional_data() -> TestResult {
        let local_addr = "127.0.0.1:0".parse()?;
        let mut listener = UdpTransportListener::bind(local_addr, config(), 1).await?;
        let mut client = connect(listener.local_addr()?, config()).await?;
        let mut accepted = tokio::time::timeout(Duration::from_secs(1), listener.accept())
            .await?
            .ok_or_else(|| std::io::Error::other("listener stopped before accepting a peer"))?;

        client.write_all(b"client").await?;
        let mut client_message = [0; 6];
        accepted.stream.read_exact(&mut client_message).await?;
        assert_eq!(&client_message, b"client");

        accepted.stream.write_all(b"server").await?;
        let mut server_message = [0; 6];
        client.read_exact(&mut server_message).await?;
        assert_eq!(&server_message, b"server");

        client.close().await?;
        accepted.stream.close().await?;
        listener.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn client_fragments_writes_using_the_negotiated_mtu() -> TestResult {
        let local_addr = "127.0.0.1:0".parse()?;
        let mut server_config = config();
        server_config.mtu = 1132;
        let mut listener = UdpTransportListener::bind(local_addr, server_config, 1).await?;
        let mut client = connect(listener.local_addr()?, config()).await?;
        let mut accepted = tokio::time::timeout(Duration::from_secs(1), listener.accept())
            .await?
            .ok_or_else(|| std::io::Error::other("listener stopped before accepting a peer"))?;

        let payload = vec![0xA5; 1200];
        client.write_all(&payload).await?;
        let mut received = vec![0; payload.len()];
        tokio::time::timeout(Duration::from_secs(1), accepted.stream.read_exact(&mut received)).await??;
        assert_eq!(received, payload);

        client.close().await?;
        accepted.stream.close().await?;
        listener.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn client_uses_the_minimum_mtu_until_the_syn_ack_arrives() -> TestResult {
        let local_addr = "127.0.0.1:0".parse()?;
        let mut server_config = config();
        server_config.mtu = u16::try_from(MIN_MTU).expect("minimum RDP-UDP MTU fits in u16");
        let mut listener = UdpTransportListener::bind(local_addr, server_config, 1).await?;
        let mut client = connect(listener.local_addr()?, config()).await?;
        assert_eq!(client.max_write_size.load(Ordering::Acquire), MIN_MTU);

        let payload = vec![0xA5; MIN_MTU + 1];
        client.write_all(&payload).await?;
        let mut accepted = tokio::time::timeout(Duration::from_secs(1), listener.accept())
            .await?
            .ok_or_else(|| std::io::Error::other("listener stopped before accepting a peer"))?;
        let mut received = vec![0; payload.len()];
        tokio::time::timeout(Duration::from_secs(1), accepted.stream.read_exact(&mut received)).await??;
        assert_eq!(received, payload);

        client.close().await?;
        accepted.stream.close().await?;
        listener.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn full_inbound_queue_does_not_block_outbound_processing() -> TestResult {
        let local_addr = "127.0.0.1:0".parse()?;
        let mut listener = UdpTransportListener::bind(local_addr, config(), 1).await?;
        let mut client = connect(listener.local_addr()?, config()).await?;
        let mut accepted = tokio::time::timeout(Duration::from_secs(1), listener.accept())
            .await?
            .ok_or_else(|| std::io::Error::other("listener stopped before accepting a peer"))?;

        for byte in 0..u8::try_from(CHANNEL_CAPACITY).unwrap() {
            client.write_all(&[byte]).await?;
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while accepted.stream.inbound_rx.len() < CHANNEL_CAPACITY {
                tokio::task::yield_now().await;
            }
        })
        .await?;

        client.write_all(&[u8::try_from(CHANNEL_CAPACITY).unwrap()]).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;

        accepted.stream.write_all(b"server").await?;
        let mut received = [0; 6];
        tokio::time::timeout(Duration::from_secs(1), client.read_exact(&mut received)).await??;
        assert_eq!(&received, b"server");

        let mut queued = [0; CHANNEL_CAPACITY + 1];
        tokio::time::timeout(Duration::from_secs(1), accepted.stream.read_exact(&mut queued)).await??;
        assert_eq!(queued, core::array::from_fn(|index| u8::try_from(index).unwrap()));

        client.close().await?;
        accepted.stream.close().await?;
        listener.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn close_drains_messages_already_accepted_from_the_writer() -> TestResult {
        let local_addr = "127.0.0.1:0".parse()?;
        let mut listener = UdpTransportListener::bind(local_addr, config(), 1).await?;
        let mut client = connect(listener.local_addr()?, config()).await?;
        let mut accepted = tokio::time::timeout(Duration::from_secs(1), listener.accept())
            .await?
            .ok_or_else(|| std::io::Error::other("listener stopped before accepting a peer"))?;

        let payload = b"accepted before close";
        client.write_all(payload).await?;
        let close = tokio::spawn(client.close());

        let mut received = vec![0; payload.len()];
        tokio::time::timeout(Duration::from_secs(1), accepted.stream.read_exact(&mut received)).await??;
        assert_eq!(received, payload);
        tokio::time::timeout(Duration::from_secs(1), close).await???;

        accepted.stream.close().await?;
        listener.close().await?;
        Ok(())
    }

    #[test]
    fn malformed_datagrams_do_not_stop_a_live_transport() -> TestResult {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut transport = ReliableUdp::new(config()).map_err(UdpTransportError::from)?;
        transport.start(now).map_err(UdpTransportError::from)?;

        assert!(handle_incoming_datagram(&mut transport, now, &[])?.is_none());
        assert_eq!(transport.state(), State::SynSent);
        Ok(())
    }

    #[tokio::test]
    async fn reader_reports_a_driver_failure() -> TestResult {
        let (inbound_tx, inbound_rx) = mpsc::channel(1);
        drop(inbound_tx);
        let (outbound_tx, _outbound_rx) = mpsc::channel(1);
        let (read_progress_tx, _read_progress_rx) = mpsc::unbounded_channel();
        let driver = tokio::spawn(async { Err(UdpTransportError::Engine(EngineError::PeerTimedOut)) });
        let mut stream = UdpTransportStream {
            inbound_rx,
            inbound_buffer: Vec::new(),
            inbound_offset: 0,
            read_progress_tx,
            outbound_tx: Some(outbound_tx),
            pending_write: None,
            shutdown_tx: None,
            driver: Some(driver),
            max_write_size: Arc::new(AtomicUsize::new(1200)),
            peer_cleanup: None,
        };

        let mut buffer = [0; 1];
        let error = stream
            .read(&mut buffer)
            .await
            .expect_err("driver failure must not appear as EOF");
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        Ok(())
    }
}
