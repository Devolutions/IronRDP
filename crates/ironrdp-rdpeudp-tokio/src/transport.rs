//! Public API: `UdpTransport`, `connect_udp()`, and `accept_udp()`.
//!
//! `connect_udp()` orchestrates the full client-side connection sequence:
//! UDP SYN/SYN+ACK/ACK → TLS handshake → RDPEMT CreateRequest/Response
//! → spawns data pump → returns `UdpTransport` handle.
//!
//! `accept_udp()` orchestrates the server-side accept sequence:
//! receive SYN → SYN+ACK/ACK → TLS accept → RDPEMT CreateResponse
//! → spawns data pump → returns `UdpTransport` handle.
//!
//! `UdpTransport` provides bidirectional higher-layer data transfer
//! (DVC frames) through the tunnel, decoupled from all the protocol
//! machinery running in background tasks.

use crate::clock::Clock;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::time::Duration;
use std::sync::{Arc, Mutex};

use ironrdp_rdpemt::{RdpemtErrorExt as _, TunnelConfig};
use ironrdp_rdpeudp::pdu::V1Datagram;
use ironrdp_rdpeudp::{ConnectionConfig, RdpeudpConnection, RdpeudpErrorExt as _};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio::net::UdpSocket;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::driver::Driver;
use crate::error::{DriverError, DriverErrorExt as _, DriverErrorKind, UdpTransportError, UdpTransportErrorExt as _};
use crate::stream::{RdpeudpStream, SharedIo};
use crate::tls::{tls_accept, tls_upgrade};
use crate::tunnel::{read_tunnel_pdu, tunnel_data_loop, write_tunnel_pdu};

// ════════════════════════════════════════════════════════════════════
// Configuration
// ════════════════════════════════════════════════════════════════════

/// Configuration for establishing a UDP transport to an RDP server.
///
/// The caller must have already received the Initiate Multitransport
/// Request PDU from the TCP connection to obtain the `tunnel_config`
/// (request_id + security_cookie).
#[derive(Clone)]
pub struct UdpTransportConfig {
    /// Server address (same IP as the TCP connection, same or different port).
    pub server_addr: SocketAddr,
    /// TLS server name for SNI.
    pub server_name: String,
    /// RDPEMT tunnel parameters from the Initiate Multitransport Request PDU.
    pub tunnel_config: TunnelConfig,
    /// RDPEUDP2 connection parameters (MTU, window size, timeouts).
    /// Uses sensible defaults if not customized.
    pub connection_config: ConnectionConfig,
    /// Maximum time to wait for the RDPEUDP2 handshake to complete.
    /// Default: 10 seconds (matching FreeRDP).
    pub handshake_timeout: Duration,
    /// How the server's TLS certificate is validated.
    ///
    /// `None` (the default) preserves the historic behavior: no
    /// verification, since RDP servers commonly present self-signed
    /// certificates. `Some(verifier)` opts into real validation, mirroring
    /// the choice `ironrdp-tls` exposes on the TCP path.
    pub server_cert_verifier: Option<Arc<dyn tokio_rustls::rustls::client::danger::ServerCertVerifier>>,
}

impl UdpTransportConfig {
    /// Create a config with the required parameters, using defaults
    /// for everything else.
    pub fn new(server_addr: SocketAddr, server_name: String, tunnel_config: TunnelConfig) -> Self {
        Self {
            server_addr,
            server_name,
            tunnel_config,
            connection_config: ConnectionConfig::default(),
            handshake_timeout: Duration::from_secs(10),
            server_cert_verifier: None,
        }
    }
}

impl core::fmt::Debug for UdpTransportConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UdpTransportConfig")
            .field("server_addr", &self.server_addr)
            .field("server_name", &self.server_name)
            .field("tunnel_config", &self.tunnel_config)
            .field("connection_config", &self.connection_config)
            .field("handshake_timeout", &self.handshake_timeout)
            .field(
                "server_cert_verifier",
                &self.server_cert_verifier.as_ref().map(|_| "Some(..)").unwrap_or("None"),
            )
            .finish()
    }
}

/// Configuration for accepting a UDP transport connection from a client.
///
/// Used by test harnesses or server implementations that need to
/// accept the client-side RDPEUDP2 connection.
pub struct UdpAcceptConfig {
    /// TLS server configuration (certificate + private key).
    pub tls_config: Arc<tokio_rustls::rustls::ServerConfig>,
    /// RDPEMT tunnel parameters: the expected request_id and
    /// security_cookie that the client will present in the
    /// Tunnel Create Request.
    pub tunnel_config: TunnelConfig,
    /// RDPEUDP2 connection parameters (MTU, window size, timeouts).
    pub connection_config: ConnectionConfig,
    /// Maximum time to wait for the full accept sequence.
    pub accept_timeout: Duration,
}

// ════════════════════════════════════════════════════════════════════
// UdpTransport
// ════════════════════════════════════════════════════════════════════

/// A spawned task that is aborted if its handle is dropped without being
/// taken back.
///
/// Dropping a bare `JoinHandle` detaches the task rather than stopping it, so
/// every early return between spawning the driver and handing it to the caller
/// used to leave it running: still holding the UDP socket, still arming
/// timers, still answering keep-alives, for the life of the process. Wrapping
/// the handle makes each `?` clean up on the way out instead of relying on
/// every path remembering to.
struct AbortOnDrop<T>(Option<JoinHandle<T>>);

impl<T> AbortOnDrop<T> {
    fn new(handle: JoinHandle<T>) -> Self {
        Self(Some(handle))
    }

    /// Take the handle back so it can be awaited instead of aborted.
    fn take(&mut self) -> Option<JoinHandle<T>> {
        self.0.take()
    }

    fn is_finished(&self) -> bool {
        self.0.as_ref().is_some_and(JoinHandle::is_finished)
    }

    /// Take the handle back only if the task has already ended, so the result
    /// can be inspected rather than discarded.
    fn take_if_finished(&mut self) -> Option<JoinHandle<T>> {
        if self.is_finished() { self.0.take() } else { None }
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        if let Some(handle) = self.0.as_ref() {
            handle.abort();
        }
    }
}

/// Handle to an established UDP transport.
///
/// Provides bidirectional higher-layer data (DVC frames) over the
/// RDPEUDP2 + TLS + RDPEMT tunnel. The protocol machinery runs in
/// background tasks; this handle communicates via channels.
///
/// Drop this handle to initiate shutdown of the background tasks.
pub struct UdpTransport {
    /// Receives higher-layer data (DVC frames) from the tunnel.
    data_rx: mpsc::Receiver<Vec<u8>>,

    /// Sends higher-layer data into the tunnel for encryption and transmission.
    data_tx: mpsc::Sender<Vec<u8>>,

    /// Shared I/O bridge between the driver and the TLS/RDPEMT layer.
    /// Held here so `shutdown()` can signal closure to both the
    /// driver (via `write_waker`) and the read pump (via `read_waker`).
    shared: Arc<Mutex<SharedIo>>,

    /// Handle to the UDP driver task (RDPEUDP2 I/O loop).
    driver_handle: AbortOnDrop<Result<(), DriverError>>,

    /// Handle to the data pump task (TLS read → RDPEMT decode → channel).
    pump_handle: AbortOnDrop<Result<(), UdpTransportError>>,

    /// Handle to the write pump task (channel → RDPEMT encode → TLS write).
    write_pump_handle: AbortOnDrop<Result<(), UdpTransportError>>,
}

impl UdpTransport {
    /// Receive the next higher-layer data frame from the tunnel.
    ///
    /// Returns `None` when the tunnel is closed.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.data_rx.recv().await
    }

    /// Send a higher-layer data frame through the tunnel.
    ///
    /// The data is wrapped in an RDPEMT TunnelData PDU, encrypted
    /// with TLS, and transmitted over the RDPEUDP2 connection.
    ///
    /// # Errors
    ///
    /// Returns `PayloadTooLarge` if `data` exceeds 65535 bytes, the wire
    /// `PayloadLength` field's capacity ([MS-RDPEMT] 2.2.2.3). Checked here,
    /// synchronously, rather than left for the background write pump to
    /// discover: that task has no way to report a per-payload failure back
    /// to a caller who already received `Ok(())` from a channel send.
    pub async fn send(&self, data: Vec<u8>) -> Result<(), UdpTransportError> {
        if data.len() > usize::from(u16::MAX) {
            return Err(UdpTransportError::payload_too_large("send", data.len()));
        }

        self.data_tx
            .send(data)
            .await
            .map_err(|_| UdpTransportError::driver("send", DriverError::connection_closed("send")))
    }

    /// Shut down the transport, closing the RDPEUDP2 connection.
    ///
    /// Signals the shared I/O bridge as closed, which propagates to
    /// both the driver (exits the select! loop) and the read pump
    /// (gets EOF from the TLS stream); the write pump stops on its own
    /// once the send channel is dropped, below. Then waits for all
    /// three background tasks to complete.
    pub async fn shutdown(mut self) -> Result<(), UdpTransportError> {
        // Drop the send channel to signal the write pump to stop
        drop(self.data_tx);
        drop(self.data_rx);

        // Signal the shared I/O bridge as closed so the driver
        // and read pump can exit cleanly.
        {
            let mut shared = self
                .shared
                .lock()
                .map_err(|_| UdpTransportError::driver_panic("shutdown"))?;
            shared.closed = true;
            if let Some(waker) = shared.read_waker.take() {
                waker.wake();
            }
            if let Some(waker) = shared.write_waker.take() {
                waker.wake();
            }
        }

        // Wait for the pump task (should now get EOF and exit)
        if let Some(pump) = self.pump_handle.take() {
            match pump.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_join_err) => return Err(UdpTransportError::driver_panic("shutdown")),
            }
        }

        // Wait for the write pump (already exited once data_tx was dropped above)
        if let Some(write_pump) = self.write_pump_handle.take() {
            match write_pump.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_join_err) => return Err(UdpTransportError::driver_panic("shutdown")),
            }
        }

        // Abort the driver (it may still be running the select! loop)
        let Some(driver) = self.driver_handle.take() else {
            return Ok(());
        };

        driver.abort();
        match driver.await {
            Ok(Ok(())) | Err(_) => Ok(()),
            Ok(Err(e)) if matches!(e.kind(), DriverErrorKind::ConnectionClosed) => Ok(()),
            Ok(Err(e)) => Err(UdpTransportError::driver("shutdown", e)),
        }
    }

    /// Whether the driver task is still running.
    pub fn is_alive(&self) -> bool {
        !self.driver_handle.is_finished()
    }

    /// Create a `UdpTransport` from raw channels, without background tasks.
    ///
    /// For unit tests that exercise the channel-based API (FramedRead,
    /// FramedWrite) without needing a real UDP socket or TLS stack.
    #[cfg(test)]
    pub(crate) fn from_channels(data_rx: mpsc::Receiver<Vec<u8>>, data_tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            data_rx,
            data_tx,
            shared: Arc::new(Mutex::new(SharedIo::new())),
            driver_handle: AbortOnDrop::new(tokio::spawn(async { Ok(()) })),
            pump_handle: AbortOnDrop::new(tokio::spawn(async { Ok(()) })),
            write_pump_handle: AbortOnDrop::new(tokio::spawn(async { Ok(()) })),
        }
    }
}

impl core::fmt::Debug for UdpTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UdpTransport")
            .field("driver_alive", &!self.driver_handle.is_finished())
            .field("pump_alive", &!self.pump_handle.is_finished())
            .field("write_pump_alive", &!self.write_pump_handle.is_finished())
            .finish()
    }
}

// ════════════════════════════════════════════════════════════════════
// connect_udp
// ════════════════════════════════════════════════════════════════════

/// Establish a UDP transport to an RDP server.
///
/// Performs the complete connection sequence:
/// 1. Binds a UDP socket and performs the RDPEUDP2 three-way handshake
/// 2. Performs a TLS handshake over the RDPEUDP2 connection
/// 3. Negotiates the RDPEMT tunnel (CreateRequest/CreateResponse)
/// 4. Spawns background data pump tasks
/// 5. Returns a `UdpTransport` handle for bidirectional data transfer
///
/// # Errors
///
/// Returns `UdpTransportError` if any phase of the connection fails:
/// socket binding, RDPEUDP2 handshake, TLS negotiation, or RDPEMT tunnel
/// rejection.
pub async fn connect_udp(config: UdpTransportConfig) -> Result<UdpTransport, UdpTransportError> {
    // Phase 1: Bind UDP socket and connect to server.
    // The bind address's family must match server_addr's: an IPv4 socket cannot
    // connect() to an IPv6 destination. Mirrors the TCP listener's family match
    // in ironrdp-server/src/server.rs (RdpServer::run).
    let bind_addr = match config.server_addr {
        SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket = UdpSocket::bind(bind_addr)
        .await
        .map_err(|error| UdpTransportError::socket("connect udp", error))?;
    socket
        .connect(config.server_addr)
        .await
        .map_err(|error| UdpTransportError::socket("connect udp", error))?;

    tracing::debug!(
        server = %config.server_addr,
        "UDP socket bound, starting RDPEUDP2 handshake"
    );

    // Phase 2: Create the sans-I/O connection and shared I/O bridge
    let shared = Arc::new(Mutex::new(SharedIo::new()));
    let connected_notify = Arc::new(Notify::new());

    let mut connection_config = config.connection_config;
    connection_config.cookie_hash = Some(cookie_hash(&config.tunnel_config));

    let conn = RdpeudpConnection::connect(connection_config, Clock::new().now()).map_err(|error| {
        UdpTransportError::handshake("connect UDP", DriverError::rdpeudp("build RDP-UDP connection", error))
    })?;
    let driver = Driver::new(socket, conn, Arc::clone(&shared), Arc::clone(&connected_notify));

    // Spawn the driver task (it immediately sends the SYN packet)
    // Guarded from here on, so the TLS and tunnel steps below abort the driver
    // if they fail rather than detaching it.
    let mut driver_handle = AbortOnDrop::new(tokio::spawn(async move { driver.run().await }));

    // Wait for the RDPEUDP2 handshake to complete (Event::Connected)
    let handshake_result = tokio::time::timeout(config.handshake_timeout, connected_notify.notified()).await;

    if handshake_result.is_err() {
        return Err(UdpTransportError::handshake_timeout("connect udp"));
    }

    // Check if driver died during handshake
    if let Some(driver) = driver_handle.take_if_finished() {
        return match driver.await {
            Ok(Ok(())) => Err(UdpTransportError::handshake(
                "connect udp",
                DriverError::connection_closed("connect udp"),
            )),
            Ok(Err(e)) => Err(UdpTransportError::handshake("connect udp", e)),
            Err(_) => Err(UdpTransportError::driver_panic("connect udp")),
        };
    }

    tracing::debug!("RDPEUDP2 handshake complete, starting TLS");

    // Phase 3: TLS handshake over the RDPEUDP2 stream
    let rdpeudp_stream = RdpeudpStream::new(Arc::clone(&shared));
    let tls_stream = tls_upgrade(rdpeudp_stream, &config.server_name, config.server_cert_verifier)
        .await
        .map_err(|error| UdpTransportError::tls("connect udp", error))?;

    tracing::debug!("TLS handshake complete, starting RDPEMT tunnel negotiation");

    // Phase 4: RDPEMT tunnel handshake
    //
    // Split the TLS stream so we can read and write concurrently.
    // tokio::io::split gives us independent read/write halves.
    let (mut tls_read, mut tls_write) = tokio::io::split(tls_stream);

    let mut tunnel = establish_tunnel_split(&mut tls_read, &mut tls_write, config.tunnel_config).await?;

    tracing::debug!("RDPEMT tunnel established, starting data pump");

    // Phase 5: Set up data channels and spawn the read pump
    let (incoming_tx, incoming_rx) = mpsc::channel::<Vec<u8>>(64);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Vec<u8>>(64);

    // Read pump: TLS → RDPEMT decode → channel
    let pump_handle = AbortOnDrop::new(tokio::spawn(async move {
        tunnel_data_loop(&mut tls_read, &mut tunnel, &incoming_tx).await
    }));

    // Write pump: channel → RDPEMT encode → TLS
    let write_pump_handle = AbortOnDrop::new(tokio::spawn(async move {
        write_pump(&mut tls_write, &mut outgoing_rx).await
    }));

    Ok(UdpTransport {
        data_rx: incoming_rx,
        data_tx: outgoing_tx,
        shared,
        driver_handle,
        pump_handle,
        write_pump_handle,
    })
}

/// Establish the RDPEMT tunnel using already-split TLS read/write halves.
async fn establish_tunnel_split<R, W>(
    reader: &mut R,
    writer: &mut W,
    config: TunnelConfig,
) -> Result<ironrdp_rdpemt::RdpemtTunnel, UdpTransportError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut tunnel = ironrdp_rdpemt::RdpemtTunnel::client(config);

    // Send CreateRequest
    if let Some(pdu_bytes) = tunnel.poll_pdu() {
        write_tunnel_pdu(writer, &pdu_bytes).await?;
    }

    // Read CreateResponse. The peer closing before it arrives is a failed
    // handshake, not a legitimate clean shutdown (there is no established
    // tunnel yet for a clean shutdown to end).
    let response = read_tunnel_pdu(reader).await?.ok_or_else(|| {
        UdpTransportError::tls(
            "establish tunnel split",
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed before tunnel response",
            ),
        )
    })?;
    tunnel
        .handle_pdu(&response)
        .map_err(|error| UdpTransportError::rdpemt("establish tunnel split", error))?;

    match tunnel.poll_event() {
        Some(ironrdp_rdpemt::TunnelEvent::Established) => Ok(tunnel),
        Some(ironrdp_rdpemt::TunnelEvent::Failed { hr_response }) => Err(UdpTransportError::tunnel_rejected(
            "establish tunnel split",
            hr_response,
        )),
        _ => Err(UdpTransportError::rdpemt(
            "establish tunnel split",
            ironrdp_rdpemt::RdpemtError::invalid_state("establish tunnel split"),
        )),
    }
}

// ════════════════════════════════════════════════════════════════════
// accept_udp (server side)
// ════════════════════════════════════════════════════════════════════

/// Accept a UDP transport connection from a client.
///
/// Performs the server-side connection sequence:
/// 1. Receives the initial SYN datagram from the pre-bound socket
/// 2. Performs the RDPEUDP2 handshake (SYN+ACK → ACK)
/// 3. Performs a TLS server-side handshake
/// 4. Handles the RDPEMT tunnel negotiation (CreateRequest → CreateResponse)
/// 5. Spawns background data pump tasks
/// 6. Returns a `UdpTransport` handle for bidirectional data transfer
///
/// The caller must provide a pre-bound `UdpSocket`. After the first
/// datagram is received, the socket is `connect()`ed to the client's
/// address so all subsequent I/O is scoped to that peer.
///
/// # Errors
///
/// Returns `UdpTransportError` if any phase of the accept fails.
pub async fn accept_udp(socket: UdpSocket, config: UdpAcceptConfig) -> Result<UdpTransport, UdpTransportError> {
    // Wrap the entire accept sequence in a timeout
    tokio::time::timeout(config.accept_timeout, accept_udp_inner(socket, config))
        .await
        .map_err(|_| UdpTransportError::handshake_timeout("accept udp"))?
}

async fn accept_udp_inner(socket: UdpSocket, config: UdpAcceptConfig) -> Result<UdpTransport, UdpTransportError> {
    // Phase 1: Receive the initial SYN datagram and learn the client address
    let mut recv_buf = vec![0u8; 9000];
    let (n, client_addr) = socket
        .recv_from(&mut recv_buf)
        .await
        .map_err(|error| UdpTransportError::socket("accept udp inner", error))?;

    tracing::debug!(client = %client_addr, "Received initial datagram, decoding SYN");

    // Connect the socket to the client so the driver can use send/recv
    socket
        .connect(client_addr)
        .await
        .map_err(|error| UdpTransportError::socket("accept udp inner", error))?;

    // Phase 2: Decode the V1 SYN datagram and create a server-side connection
    let syn_datagram: V1Datagram = ironrdp_core::decode(&recv_buf[..n]).map_err(|_| {
        UdpTransportError::handshake(
            "accept UDP",
            DriverError::rdpeudp(
                "decode SYN datagram",
                ironrdp_rdpeudp::RdpeudpError::invalid_packet("decode SYN datagram", "SYN datagram did not decode"),
            ),
        )
    })?;

    let mut connection_config = config.connection_config;
    connection_config.cookie_hash = Some(cookie_hash(&config.tunnel_config));

    let conn = RdpeudpConnection::accept(connection_config, &syn_datagram, Clock::new().now()).map_err(|error| {
        UdpTransportError::handshake("accept UDP", DriverError::rdpeudp("accept RDP-UDP connection", error))
    })?;

    // Phase 3: Start the driver (it picks up the SYN+ACK from poll_transmit)
    let shared = Arc::new(Mutex::new(SharedIo::new()));
    let connected_notify = Arc::new(Notify::new());

    let driver = Driver::new(socket, conn, Arc::clone(&shared), Arc::clone(&connected_notify));
    // Guarded from here on, so the TLS and tunnel steps below abort the driver
    // if they fail rather than detaching it.
    let mut driver_handle = AbortOnDrop::new(tokio::spawn(async move { driver.run().await }));

    // Wait for the RDPEUDP2 handshake to complete (ACK received)
    connected_notify.notified().await;

    if let Some(driver) = driver_handle.take_if_finished() {
        return match driver.await {
            Ok(Ok(())) => Err(UdpTransportError::handshake(
                "accept udp inner",
                DriverError::connection_closed("accept udp inner"),
            )),
            Ok(Err(e)) => Err(UdpTransportError::handshake("accept udp inner", e)),
            Err(_) => Err(UdpTransportError::driver_panic("accept udp inner")),
        };
    }

    tracing::debug!("RDPEUDP2 server handshake complete, starting TLS accept");

    // Phase 4: TLS server-side handshake
    let rdpeudp_stream = RdpeudpStream::new(Arc::clone(&shared));
    let tls_stream = tls_accept(rdpeudp_stream, config.tls_config)
        .await
        .map_err(|error| UdpTransportError::tls("accept udp inner", error))?;

    tracing::debug!("TLS accept complete, starting RDPEMT tunnel negotiation");

    // Phase 5: RDPEMT server-side tunnel handshake
    let (mut tls_read, mut tls_write) = tokio::io::split(tls_stream);

    let mut tunnel = establish_tunnel_server_split(&mut tls_read, &mut tls_write, config.tunnel_config).await?;

    tracing::debug!("RDPEMT server tunnel established, starting data pump");

    // Phase 6: Set up data channels and spawn pumps (identical to client side)
    let (incoming_tx, incoming_rx) = mpsc::channel::<Vec<u8>>(64);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Vec<u8>>(64);

    let pump_handle = AbortOnDrop::new(tokio::spawn(async move {
        tunnel_data_loop(&mut tls_read, &mut tunnel, &incoming_tx).await
    }));

    // Write pump: channel → RDPEMT encode → TLS
    let write_pump_handle = AbortOnDrop::new(tokio::spawn(async move {
        write_pump(&mut tls_write, &mut outgoing_rx).await
    }));

    Ok(UdpTransport {
        data_rx: incoming_rx,
        data_tx: outgoing_tx,
        shared,
        driver_handle,
        pump_handle,
        write_pump_handle,
    })
}

/// Server-side RDPEMT tunnel establishment.
///
/// Mirrors `establish_tunnel_split` but uses `RdpemtTunnel::server()`:
/// reads the client's CreateRequest, validates it, sends CreateResponse.
async fn establish_tunnel_server_split<R, W>(
    reader: &mut R,
    writer: &mut W,
    config: TunnelConfig,
) -> Result<ironrdp_rdpemt::RdpemtTunnel, UdpTransportError>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut tunnel = ironrdp_rdpemt::RdpemtTunnel::server(config);

    // Read CreateRequest from client. A close before it arrives is a failed
    // handshake, not a legitimate clean shutdown.
    let request = read_tunnel_pdu(reader).await?.ok_or_else(|| {
        UdpTransportError::tls(
            "establish tunnel server split",
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed before tunnel request",
            ),
        )
    })?;
    tunnel
        .handle_pdu(&request)
        .map_err(|error| UdpTransportError::rdpemt("establish tunnel server split", error))?;

    // Send CreateResponse
    if let Some(pdu_bytes) = tunnel.poll_pdu() {
        write_tunnel_pdu(writer, &pdu_bytes).await?;
    }

    match tunnel.poll_event() {
        Some(ironrdp_rdpemt::TunnelEvent::Established) => Ok(tunnel),
        Some(ironrdp_rdpemt::TunnelEvent::Failed { hr_response }) => Err(UdpTransportError::tunnel_rejected(
            "establish tunnel server split",
            hr_response,
        )),
        _ => Err(UdpTransportError::rdpemt(
            "establish tunnel server side",
            ironrdp_rdpemt::RdpemtError::invalid_state("establish tunnel server side"),
        )),
    }
}

/// Write pump: drains outgoing channel → RDPEMT encode → TLS write.
///
/// Shared by both `connect_udp` and `accept_udp`. Returns the first failure
/// encountered rather than only logging it, so `shutdown()` can surface it
/// to the caller instead of the pump silently going quiet.
async fn write_pump<W>(tls_write: &mut W, outgoing_rx: &mut mpsc::Receiver<Vec<u8>>) -> Result<(), UdpTransportError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    while let Some(data) = outgoing_rx.recv().await {
        let pdu = ironrdp_rdpemt::TunnelData {
            sub_headers: Vec::new(),
            higher_layer_data: data,
        };
        let encoded = ironrdp_core::encode_vec(&pdu)
            .map_err(|error| UdpTransportError::rdpemt("write pump", ironrdp_rdpemt::RdpemtError::encode(error)))?;
        tls_write
            .write_all(&encoded)
            .await
            .map_err(|error| UdpTransportError::tls("write pump", error))?;
        tls_write
            .flush()
            .await
            .map_err(|error| UdpTransportError::tls("write pump", error))?;
    }
    Ok(())
}

/// The value a version 3 SYN binds itself to: the SHA-256 of the
/// `securityCookie` the server sent in the Initiate Multitransport Request
/// PDU ([MS-RDPEUDP] 2.2.2.9).
///
/// Derived here rather than asked of the caller, so the hash in the handshake
/// is always over the same cookie the RDPEMT tunnel will present a moment
/// later. The sans-I/O crate takes the finished hash and stays free of any
/// cryptographic dependency.
fn cookie_hash(tunnel_config: &TunnelConfig) -> [u8; 32] {
    Sha256::digest(tunnel_config.security_cookie).into()
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// Dropping the guard stops the task rather than detaching it.
    ///
    /// A bare `JoinHandle` detaches on drop, so every early return between
    /// spawning the driver and returning a `UdpTransport` used to leave it
    /// running with the socket, the timers and the keep-alives.
    #[tokio::test]
    async fn the_guard_aborts_a_task_it_still_owns() {
        static STILL_RUNNING: AtomicBool = AtomicBool::new(false);
        STILL_RUNNING.store(false, Ordering::SeqCst);

        let guard = AbortOnDrop::new(tokio::spawn(async {
            loop {
                STILL_RUNNING.store(true, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }));

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(STILL_RUNNING.load(Ordering::SeqCst), "the task should have started");

        drop(guard);
        tokio::time::sleep(Duration::from_millis(20)).await;

        STILL_RUNNING.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(
            !STILL_RUNNING.load(Ordering::SeqCst),
            "the task outlived the guard that owned it"
        );
    }

    /// Taking the handle back disarms the guard, so a task that is about to be
    /// awaited is not killed underneath the caller.
    #[tokio::test]
    async fn taking_the_handle_back_disarms_the_guard() {
        let mut guard = AbortOnDrop::new(tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            7u8
        }));

        let handle = guard.take().expect("the handle is still there");
        drop(guard);

        assert_eq!(handle.await.expect("the task should not have been aborted"), 7);
    }

    /// Dropping a `UdpTransport` stops its background tasks. Without this the
    /// driver keeps the socket open and keeps answering keep-alives for the
    /// life of the process.
    #[tokio::test]
    async fn dropping_the_transport_stops_its_tasks() {
        static DRIVER_RUNNING: AtomicBool = AtomicBool::new(false);
        DRIVER_RUNNING.store(false, Ordering::SeqCst);

        let (_incoming_tx, incoming_rx) = mpsc::channel::<Vec<u8>>(4);
        let (outgoing_tx, _outgoing_rx) = mpsc::channel::<Vec<u8>>(4);

        let transport = UdpTransport {
            data_rx: incoming_rx,
            data_tx: outgoing_tx,
            shared: Arc::new(Mutex::new(SharedIo::new())),
            driver_handle: AbortOnDrop::new(tokio::spawn(async {
                loop {
                    DRIVER_RUNNING.store(true, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })),
            pump_handle: AbortOnDrop::new(tokio::spawn(async { Ok(()) })),
            write_pump_handle: AbortOnDrop::new(tokio::spawn(async { Ok(()) })),
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(DRIVER_RUNNING.load(Ordering::SeqCst));

        drop(transport);
        tokio::time::sleep(Duration::from_millis(20)).await;

        DRIVER_RUNNING.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(
            !DRIVER_RUNNING.load(Ordering::SeqCst),
            "the driver outlived the transport that owned it"
        );
    }
}
