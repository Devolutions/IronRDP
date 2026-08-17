//! RDPEUDP2 connection state machine.
//!
//! Sans-I/O design: the state machine never touches the network or system
//! clock. All `MonotonicInstant` values are passed in by the caller, and outgoing
//! packets are returned as `Transmit` values. This mirrors Quinn's
//! `Connection` architecture (ADR-0001).
//!
//! # Lifecycle
//!
//! ```text
//! Client: connect() → SynSent ──(recv SYN+ACK)──→ Established
//! Server: accept()  → SynReceived ──(recv ACK)──→ Established
//! Either: close()   → Closed
//! Either: idle timeout ──→ Closed
//! ```
//!
//! # V1/V2 packet discrimination
//!
//! The v1 wire format is used for the three-way handshake (SYN/SYN+ACK/ACK).
//! Once established, all packets use the v2 format with the PacketPrefixByte
//! framing from MS-RDPEUDP2 Section 2.2.1.3.

use crate::time::MonotonicInstant;
use alloc::collections::VecDeque;
#[cfg(not(feature = "std"))]
use alloc::{vec, vec::Vec};
use core::time::Duration;

use ironrdp_core::{decode, encode_vec};

use crate::congestion::CongestionControl;
use crate::error::{RdpeudpError, RdpeudpErrorExt as _};
use crate::loss::LossDetector;
use crate::pdu::prefix::{decode_with_prefix, encode_with_prefix};
use crate::pdu::v1_ack::{V1AckVectorElement, V1AckVectorHeader, VectorElementState};
use crate::pdu::v1_flags::V1Flags;
use crate::pdu::v1_header::FecHeader;
use crate::pdu::v1_syn::{SynDataExPayload, SynDataPayload, SynExFlags, UdpVersion};
use crate::pdu::v2_ack::{ACK_VECTOR_MAX_ENTRIES, AckPayload, AckVectorEntry, AckVectorPayload};
use crate::pdu::v2_control::AckOfAcksPayload;
use crate::pdu::v2_data::{DataBody, DataHeader};
use crate::pdu::v2_flags::V2Flags;
use crate::pdu::v2_header::{LOG_WINDOW_SIZE_MAX, V2Header};
use crate::pdu::{V1Datagram, V2Packet};
use crate::recv_window::RecvWindow;
use crate::reliability::ReliabilityController;
use crate::rtt::RttEstimator;
use crate::send_window::SendWindow;
use crate::seq;
use crate::timer::{Timer, TimerTable};

// ════════════════════════════════════════════════════════════════════
// Public types
// ════════════════════════════════════════════════════════════════════

/// Which side of the connection this endpoint represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Initiated the connection (sent the initial SYN).
    Client,
    /// Accepted the connection (responded with SYN+ACK).
    Server,
}

/// Events produced by the state machine for the application layer.
///
/// Retrieved by calling `poll_event()` after any state mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The handshake completed and the connection is established.
    Connected,

    /// Application data has been received and reassembled in order.
    DataReceived(Vec<u8>),

    /// The connection has been closed (locally or by timeout).
    ConnectionClosed,
}

/// An outgoing packet to be sent on the wire.
///
/// Retrieved by calling `poll_transmit()` after any state mutation.
#[derive(Debug, Clone)]
pub struct Transmit {
    /// The raw wire bytes to send (including PacketPrefixByte for v2 packets).
    pub contents: Vec<u8>,
}

/// Configuration for a new RDPEUDP2 connection.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Initial sequence number from the SYN exchange.
    pub initial_sequence_number: u32,

    /// log2 of the receive window size (0..=15).
    /// Actual window = `1 << log_window_size`.
    pub log_window_size: u8,

    /// Upstream MTU (client → server), negotiated during handshake.
    pub upstream_mtu: u16,

    /// Downstream MTU (server → client), negotiated during handshake.
    pub downstream_mtu: u16,

    /// Idle timeout: connection closes if no packets received within this duration.
    ///
    /// Default: 65 seconds, the interval [MS-RDPEUDP] 3.1.1.9 gives for
    /// deciding the peer has gone. Shortening it below that closes
    /// connections a conforming peer still considers live.
    pub idle_timeout: Duration,

    /// Keep-alive interval: send a probe if no data has been sent for this long.
    /// Should be shorter than the remote's idle timeout.
    ///
    /// Default: 8 seconds. The spec asks only that endpoints acknowledge
    /// "periodically" (3.1.1.9), the point being to hold the NAT binding
    /// open, so the interval is ours to pick.
    pub keep_alive_interval: Duration,

    /// SHA-256 of the `securityCookie` from the Initiate Multitransport
    /// Request PDU ([MS-RDPBCGR] 2.2.15.1).
    ///
    /// Required, despite being an `Option`: this crate implements the
    /// MS-RDPEUDP2 data transfer, which [MS-RDPEUDP] 1.3.2.2 reaches only at
    /// protocol version 3, and 2.2.2.9 requires the hash in a version 3 SYN.
    /// A connection without it cannot be built, so [`RdpeudpConnection::connect`]
    /// and [`RdpeudpConnection::accept`] both refuse one.
    ///
    /// The default is `None` because there is no meaningful value to invent.
    /// Callers get the cookie from the multitransport request and hash it;
    /// `ironrdp-rdpeudp-tokio` does this for them.
    ///
    /// The server holds the same value and compares it against the client's
    /// SYN, which is the check 3.1.5.1.1 asks of it.
    pub cookie_hash: Option<[u8; 32]>,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            initial_sequence_number: 0,
            log_window_size: 6,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
            idle_timeout: Duration::from_secs(65),
            keep_alive_interval: Duration::from_secs(8),
            cookie_hash: None,
        }
    }
}

// ════════════════════════════════════════════════════════════════════
// Internal state
// ════════════════════════════════════════════════════════════════════

/// The RDP-UDP2 MTU.
///
/// [MS-RDPEUDP2] 1.5 fixes it: "The maximum transmission unit (MTU) size in
/// RDP-UDP2 transport layer is set to 1232 bytes." The v1 handshake still
/// negotiates `uUpStreamMtu` and `uDownStreamMtu` in the range 1132 to 1232,
/// so the effective limit is whichever of the two is smaller.
const RDPEUDP2_MTU: usize = 1232;

/// What a data packet spends on framing before any payload: the
/// PacketPrefixByte (1), the RDP-UDP2 header (2), the DataHeader (2) and the
/// DataBody's ChannelSeqNum (2).
const DATA_PACKET_OVERHEAD: usize = 7;

/// Room held back on every data packet for an acknowledgment riding along.
///
/// The largest is an ACK vector: three bytes fixed, four for the timestamp
/// block, and up to 127 entries, that being the limit of its 7-bit size field.
/// The cumulative form reaches 22 and cannot appear beside it, since 2.2.1.1
/// makes the two flags mutually exclusive. An AckOfAcks adds two more.
///
/// Reserving the worst case costs about a tenth of each packet on a link with
/// nothing to report. Measuring instead, and dropping the acknowledgment when
/// it would not fit, would buy that back at the cost of a second way for a
/// packet to be built; not worth it until something shows the throughput
/// matters. `a_full_data_packet_fits_the_mtu` checks this figure against the
/// encoders rather than leaving it as arithmetic in a comment.
const PIGGYBACK_RESERVE: usize = 134 + 2;

/// How many unanswered retransmits of a handshake datagram we tolerate
/// before closing.
///
/// [MS-RDPEUDP] 3.1.5.4.1 puts the cutoff at "at least three and no more
/// than five", so an endpoint may not give up earlier than three and must
/// have given up by five. Five is the friendliest conforming choice on a
/// link that is losing packets.
const HANDSHAKE_RETRANSMIT_LIMIT: u8 = 5;

/// How many multiples of the send window's own capacity `send()` lets the
/// send buffer hold ahead of it before returning `SendBufferFull`.
///
/// Generous enough that writing well ahead of a slow or lossy link is not
/// mistaken for the unbounded growth this bound exists to catch: this
/// crate's own tests queue up to 4x a window's worth in one burst before
/// any of it drains.
const SEND_BUFFER_WINDOW_MULTIPLE: usize = 8;

/// Connection state in the handshake / lifecycle FSM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Client has sent SYN, waiting for SYN+ACK.
    SynSent,
    /// Server has received SYN and sent SYN+ACK, waiting for final ACK.
    SynReceived,
    /// Handshake complete, data transfer active.
    Established,
    /// Connection terminated.
    Closed,
}

/// An acknowledgment ready to go onto an outgoing packet.
///
/// The cumulative and vector forms are mutually exclusive: [MS-RDPEUDP2]
/// 2.2.1.1 says the ACKVEC flag "MUST NOT be set if the ACK flag is set".
struct Acknowledgement {
    flag: V2Flags,
    ack: Option<AckPayload>,
    ack_vector: Option<AckVectorPayload>,
}

/// Negotiated parameters from the handshake.
#[derive(Debug, Clone)]
struct NegotiatedParams {
    /// Our ISN (from our SYN).
    local_isn: u32,
    /// Remote ISN (from their SYN).
    remote_isn: u32,
    /// Negotiated MTU: min(upstream, downstream, remote_upstream, remote_downstream).
    mtu: u16,
    /// log2 of the window size.
    log_window_size: u8,
}

// ════════════════════════════════════════════════════════════════════
// Connection
// ════════════════════════════════════════════════════════════════════

/// RDPEUDP2 connection state machine.
///
/// Wires together all protocol components (send/receive windows,
/// congestion control, loss detection, RTT estimation, timers,
/// reliability controller) behind a sans-I/O polling API.
///
/// # Usage pattern
///
/// ```ignore
/// // Create
/// let mut conn = RdpeudpConnection::connect(config, now);
///
/// // Main loop:
/// // 1. Send the initial SYN
/// while let Some(transmit) = conn.poll_transmit() {
///     socket.send(&transmit.contents);
/// }
///
/// // 2. Receive packets
/// conn.handle_datagram(&mut incoming_bytes, now)?;
///
/// // 3. Check for events
/// while let Some(event) = conn.poll_event() {
///     match event {
///         Event::Connected => { /* handshake done */ }
///         Event::DataReceived(data) => { /* process data */ }
///         Event::ConnectionClosed => { break; }
///     }
/// }
///
/// // 4. Handle timeouts
/// if let Some(deadline) = conn.poll_timeout() {
///     // schedule wakeup at `deadline`
/// }
/// conn.handle_timeout(now);
/// ```
pub struct RdpeudpConnection {
    /// Which side we are.
    side: Side,

    /// Current lifecycle state.
    state: State,

    /// Configuration (immutable after construction).
    config: ConnectionConfig,

    /// Negotiated handshake parameters (populated during/after handshake).
    params: Option<NegotiatedParams>,

    // ── Data transfer components (initialized on Established) ──
    send_window: Option<SendWindow>,
    recv_window: Option<RecvWindow>,
    congestion: CongestionControl,
    loss_detector: LossDetector,
    reliability: ReliabilityController,
    rtt: RttEstimator,
    timers: TimerTable,

    /// Queued outgoing wire-format packets.
    pending_transmits: VecDeque<Transmit>,

    /// Queued events for the application.
    pending_events: VecDeque<Event>,

    /// Application data waiting to be packaged into packets.
    send_buffer: VecDeque<Vec<u8>>,

    /// Whether we need to send a standalone ACK (no data piggybacked).
    ack_pending: bool,

    /// The DataSeqNum we are waiting to see acknowledged, when the peer still
    /// needs telling.
    ///
    /// [MS-RDPEUDP2] 2.2.1.2.4 defines AckOfAcksSeqNum as "the sequence number
    /// the Sender is waiting to receive acknowledgment of", and 3.1.5.3 has
    /// the sender advertise it after declaring a loss, "so that the Receiver
    /// can stop waiting for any packets with lower sequence numbers to
    /// arrive".
    ///
    /// This is the only thing that can move a receiver past a packet we gave
    /// up on. The retransmission carries a fresh DataSeqNum, so the lost one
    /// is never filled, and 3.1.1.2.2 lets a receiver advance its lower bound
    /// only over received packets or on this payload. Without it the peer's
    /// window sits on the hole until it fills, and the connection stops
    /// carrying data one window later.
    ///
    /// Held as the full 64-bit value and truncated on the wire, so it can be
    /// compared against what the peer reports.
    pending_ack_of_acks: Option<u64>,

    /// The last handshake datagram we put on the wire, kept so it can go
    /// out again. [MS-RDPEUDP] 1.3.1 delivers the SYN, the SYN+ACK and the
    /// ACK by persistent retransmits whatever mode the transport is in.
    ///
    /// The client holds on to its final ACK after establishing, because a
    /// repeated SYN+ACK is the server saying it never arrived.
    handshake_datagram: Option<Vec<u8>>,

    /// How many times `handshake_datagram` has been retransmitted by the
    /// timer. Peer-prompted resends do not count: those got an answer.
    handshake_retransmits: u8,

    /// When the last handshake datagram (`handshake_datagram`) was first
    /// sent, for sampling the handshake round trip as the connection's
    /// first RTT estimate. `None` until the first SYN or SYN+ACK goes out.
    handshake_sent_at: Option<MonotonicInstant>,

    /// When the earliest currently-unacknowledged data arrived, `None` when
    /// nothing is owed an acknowledgment.
    ///
    /// Set the first time [`process_data`]/[`process_dummy_data`] has
    /// something new to acknowledge and [`Timer::AckDelay`] is not already
    /// armed, matching that timer's own arm condition. Read by
    /// [`build_ack_payload`]/[`build_ack_vector_payload`] to report the real
    /// `received_ts` and the real gap between receipt and acknowledgment,
    /// and cleared by [`commit_acknowledgement`] once that acknowledgment has
    /// actually gone out.
    ///
    /// [`process_data`]: RdpeudpConnection::process_data
    /// [`process_dummy_data`]: RdpeudpConnection::process_dummy_data
    /// [`build_ack_payload`]: RdpeudpConnection::build_ack_payload
    /// [`build_ack_vector_payload`]: RdpeudpConnection::build_ack_vector_payload
    /// [`commit_acknowledgement`]: RdpeudpConnection::commit_acknowledgement
    ack_delay_started_at: Option<MonotonicInstant>,

    /// Remote timestamp reference for reconstruction.
    remote_timestamp_ref: u64,

    /// Reusable wire encoding buffer.
    wire_buf: Vec<u8>,
}

impl RdpeudpConnection {
    // ════════════════════════════════════════════════════════════════
    // Constructors
    // ════════════════════════════════════════════════════════════════

    /// Create a client-side connection and enqueue the initial SYN.
    ///
    /// After calling this, use `poll_transmit()` to retrieve the SYN
    /// datagram to send on the wire.
    ///
    /// Returns an error if `config.cookie_hash` is absent. The SYN advertises
    /// protocol version 3, which is what selects the MS-RDPEUDP2 data
    /// transfer this crate implements, and [MS-RDPEUDP] 2.2.2.9 requires the
    /// hash to accompany that version.
    pub fn connect(config: ConnectionConfig, now: MonotonicInstant) -> Result<Self, RdpeudpError> {
        if config.log_window_size > LOG_WINDOW_SIZE_MAX {
            return Err(RdpeudpError::invalid_state(
                "ConnectionConfig::log_window_size must be 0..=15",
            ));
        }

        if config.cookie_hash.is_none() {
            return Err(RdpeudpError::invalid_state(
                "connect without ConnectionConfig::cookie_hash, which a version 3 SYN must carry",
            ));
        }

        let mut conn = Self::new(Side::Client, config);
        conn.enqueue_syn(now);
        Ok(conn)
    }

    /// Create a server-side connection from a received SYN datagram.
    ///
    /// The caller has already decoded the V1 datagram and determined
    /// it's a SYN. This constructs the server state machine and
    /// enqueues the SYN+ACK response.
    ///
    /// Returns an error if the SYN datagram is malformed (missing
    /// SYN data or version negotiation).
    pub fn accept(
        config: ConnectionConfig,
        syn_datagram: &V1Datagram,
        now: MonotonicInstant,
    ) -> Result<Self, RdpeudpError> {
        if config.log_window_size > LOG_WINDOW_SIZE_MAX {
            return Err(RdpeudpError::invalid_state(
                "ConnectionConfig::log_window_size must be 0..=15",
            ));
        }

        let syn_data = syn_datagram
            .syn_data
            .as_ref()
            .ok_or_else(|| RdpeudpError::invalid_packet("accept", "SYN datagram missing SynData payload"))?;

        let syn_data_ex = syn_datagram
            .syn_data_ex
            .as_ref()
            .ok_or_else(|| RdpeudpError::invalid_packet("accept", "SYN datagram missing SynDataEx payload"))?;

        let expected_hash = config.cookie_hash.ok_or_else(|| {
            RdpeudpError::invalid_state(
                "accept without ConnectionConfig::cookie_hash, needed to check the client's SYN",
            )
        })?;

        // Only version 3 selects the MS-RDPEUDP2 data transfer (1.3.2.2), and
        // that is the only data transfer this crate implements, so a client
        // offering version 1 or 2 (asking for the MS-RDPEUDP one) cannot be
        // served. A client offering something above version 3 can: MS-RDPEUDP
        // 1.7's negotiate-down MUST clause requires settling on our own
        // highest supported version rather than refusing the connection, and
        // `enqueue_syn_ack` below always answers with version 3 regardless of
        // what was offered, which is exactly that settlement.
        if syn_data_ex.udp_ver.0 < UdpVersion::V3.0 {
            return Err(RdpeudpError::invalid_packet(
                "accept",
                "remote offered a protocol version below 3, whose data transfer is MS-RDPEUDP rather than MS-RDPEUDP2",
            ));
        }

        // 3.1.5.1.1 asks the server to confirm the hash, and says an invalid
        // one MUST drop the connection back to version 2. That version means
        // the MS-RDPEUDP data transfer, which this crate does not implement,
        // so the only honest outcome is to refuse the connection.
        let offered_hash = syn_data_ex
            .cookie_hash
            .ok_or_else(|| RdpeudpError::invalid_packet("accept", "version 3 SYN carries no cookieHash"))?;

        if offered_hash != expected_hash {
            return Err(RdpeudpError::invalid_packet(
                "accept",
                "cookieHash does not match the security cookie for this multitransport request",
            ));
        }

        let mut conn = Self::new(Side::Server, config);
        conn.state = State::SynReceived;

        let remote_isn = syn_data.initial_sequence_number;
        let local_isn = conn.config.initial_sequence_number;

        // Negotiate MTU: minimum of all advertised values
        let mtu = conn
            .config
            .upstream_mtu
            .min(conn.config.downstream_mtu)
            .min(syn_data.upstream_mtu)
            .min(syn_data.downstream_mtu);

        conn.params = Some(NegotiatedParams {
            local_isn,
            remote_isn,
            mtu,
            log_window_size: conn.config.log_window_size,
        });

        conn.enqueue_syn_ack(remote_isn, now);
        conn.timers.set(Timer::Idle, now + conn.config.idle_timeout);

        Ok(conn)
    }

    fn new(side: Side, config: ConnectionConfig) -> Self {
        Self {
            side,
            state: State::SynSent,
            config,
            params: None,
            send_window: None,
            recv_window: None,
            congestion: CongestionControl::new(),
            loss_detector: LossDetector::new(),
            reliability: ReliabilityController::new(),
            rtt: RttEstimator::new(),
            timers: TimerTable::new(),
            pending_transmits: VecDeque::new(),
            pending_events: VecDeque::new(),
            send_buffer: VecDeque::new(),
            ack_pending: false,
            pending_ack_of_acks: None,
            handshake_datagram: None,
            handshake_retransmits: 0,
            handshake_sent_at: None,
            ack_delay_started_at: None,
            remote_timestamp_ref: 0,
            wire_buf: Vec::with_capacity(1400),
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Public API
    // ════════════════════════════════════════════════════════════════

    /// Enqueue application data for transmission.
    ///
    /// The data will be sent in the next `poll_transmit()` call,
    /// subject to congestion window availability.
    ///
    /// Returns an error if the connection is not established or
    /// the send buffer is full.
    pub fn send(&mut self, data: Vec<u8>) -> Result<(), RdpeudpError> {
        match self.state {
            State::Established => {}
            State::Closed => return Err(RdpeudpError::connection_closed("send")),
            _ => return Err(RdpeudpError::invalid_state("send")),
        }

        let max_payload = self.max_payload();

        // Bounded at the send window's own capacity: past that, nothing
        // queued here can drain any faster than the window already allows,
        // so holding more only grows memory under sustained congestion or an
        // unresponsive peer. Checked whole against every chunk this call
        // would add before pushing any of them, since pushing part of a
        // split chunk set and rejecting the rest would corrupt the byte
        // stream the receiving end reassembles.
        let max_entries = SEND_BUFFER_WINDOW_MULTIPLE * (1usize << usize::from(self.config.log_window_size));
        let chunks_needed = data.len().div_ceil(max_payload).max(1);
        if self.send_buffer.len() + chunks_needed > max_entries {
            return Err(RdpeudpError::send_buffer_full("send"));
        }

        if data.len() <= max_payload {
            self.send_buffer.push_back(data);
            return Ok(());
        }

        // Split rather than reject. The caller here is a TLS session writing a
        // byte stream: it has no message boundaries to preserve and no notion
        // of what fits in a datagram, and the receiving end concatenates
        // whatever arrives before handing it back to TLS.
        for chunk in data.chunks(max_payload) {
            self.send_buffer.push_back(chunk.to_vec());
        }

        Ok(())
    }

    /// The largest payload this connection will put in one packet.
    ///
    /// [MS-RDPEUDP2] has no segmentation. 3.1.1.2.4.2 has the receiver strip
    /// the ChannelSeqNum and forward "the rest of the data payload to the
    /// upper layer immediately", and the format carries no first or last
    /// marker to reassemble by, so a packet's payload arrives whole or not at
    /// all. Anything longer has to be split before it becomes packets, which
    /// is what [`send`] does.
    ///
    /// [`send`]: Self::send
    pub fn max_payload(&self) -> usize {
        let mtu = self
            .params
            .as_ref()
            .map_or(RDPEUDP2_MTU, |params| usize::from(params.mtu).min(RDPEUDP2_MTU));

        mtu.saturating_sub(DATA_PACKET_OVERHEAD + PIGGYBACK_RESERVE).max(1)
    }

    /// Process a received wire-format datagram.
    ///
    /// For v1 (handshake) datagrams, pass the raw UDP payload bytes.
    /// For v2 (data) packets, pass the raw UDP payload including the
    /// PacketPrefixByte framing.
    ///
    /// The `wire` slice must be mutable because `decode_with_prefix`
    /// performs an in-place byte swap.
    pub fn handle_datagram(&mut self, wire: &mut [u8], now: MonotonicInstant) -> Result<(), RdpeudpError> {
        if self.state == State::Closed {
            return Err(RdpeudpError::connection_closed("handle datagram"));
        }

        // Reset idle timer on any received packet
        self.timers.set(Timer::Idle, now + self.config.idle_timeout);

        match self.state {
            State::SynSent => self.handle_syn_ack(wire, now),
            State::SynReceived => self.handle_final_ack(wire, now),
            State::Established => {
                // A server that missed our final ACK repeats its SYN+ACK, in
                // the v1 format, long after we have moved on to v2.
                if self.is_repeated_syn_ack(wire) {
                    self.resend_handshake_datagram();
                    return Ok(());
                }

                self.handle_v2_packet(wire, now)
            }
            State::Closed => Err(RdpeudpError::connection_closed("handle datagram")),
        }
    }

    /// Whether `wire` is the server repeating the SYN+ACK we already
    /// answered.
    ///
    /// v1 and v2 datagrams share no framing and no discriminator, so this
    /// leans on the initial sequence number: a v2 data packet would have to
    /// carry the exact 32-bit value the server opened with, at the offset
    /// SYNDATA occupies, to be mistaken for one.
    fn is_repeated_syn_ack(&self, wire: &[u8]) -> bool {
        if self.side != Side::Client {
            return false;
        }

        let Some(params) = self.params.as_ref() else {
            return false;
        };

        // Read the fixed header on its own first. This runs for every
        // datagram the connection receives once established, and a full v1
        // parse of arbitrary bytes can allocate an ACK vector of up to 65535
        // elements before deciding they were never a SYN+ACK.
        let Ok(header) = decode::<FecHeader>(wire) else {
            return false;
        };

        if !header.flags.contains(V1Flags::SYN | V1Flags::ACK) {
            return false;
        }

        let Ok(datagram) = decode::<V1Datagram>(wire) else {
            return false;
        };

        datagram
            .syn_data
            .is_some_and(|syn_data| syn_data.initial_sequence_number == params.remote_isn)
    }

    /// Retrieve the next outgoing packet to send on the wire.
    ///
    /// Returns `None` when there are no more packets to send.
    /// The caller should call this in a loop until it returns `None`.
    ///
    /// This method also generates new data packets from the send
    /// buffer when congestion window budget is available, processes
    /// retransmissions, and sends standalone ACKs.
    pub fn poll_transmit(&mut self, now: MonotonicInstant) -> Option<Transmit> {
        // A closed connection has nothing left to say. Returning early also
        // keeps the handshake branch below from arming the keep-alive timer
        // after `close` cleared it: `handle_timeout` returns early once closed,
        // so a timer armed here would stay due forever and a caller polling
        // the deadline without checking `is_closed` would spin.
        if self.state == State::Closed {
            return None;
        }

        // First, drain any pre-built transmits (handshake packets)
        if let Some(t) = self.pending_transmits.pop_front() {
            self.timers.set(Timer::KeepAlive, now + self.config.keep_alive_interval);
            return Some(t);
        }

        if self.state != State::Established {
            return None;
        }

        // Priority order:
        // 1. Retransmissions (reliability queue)
        // 2. New data from send buffer
        // 3. Standalone ACK (if needed)
        // 4. Keep-alive probe

        // Try retransmissions
        if let Some(transmit) = self.try_retransmit(now) {
            self.timers.set(Timer::KeepAlive, now + self.config.keep_alive_interval);
            return Some(transmit);
        }

        // Try new data
        if let Some(transmit) = self.try_send_new_data(now) {
            self.timers.set(Timer::KeepAlive, now + self.config.keep_alive_interval);
            return Some(transmit);
        }

        // Standalone ACK
        if self.ack_pending {
            if let Some(transmit) = self.build_standalone_ack(now) {
                self.ack_pending = false;
                self.timers.clear(Timer::AckDelay);
                self.timers.set(Timer::KeepAlive, now + self.config.keep_alive_interval);
                return Some(transmit);
            }
        }

        None
    }

    /// The next time the caller should invoke `handle_timeout()`.
    ///
    /// Returns `None` if no timers are active.
    pub fn poll_timeout(&self) -> Option<MonotonicInstant> {
        self.timers.next_deadline()
    }

    /// Process expired timers at the current time.
    ///
    /// The caller should invoke this when `now >= poll_timeout()`.
    pub fn handle_timeout(&mut self, now: MonotonicInstant) {
        if self.state == State::Closed {
            return;
        }

        let expired: Vec<Timer> = self.timers.expired(now).collect();

        for timer in expired {
            // The idle timer closes the connection, and `close` clears every
            // timer. Handlers later in this list would otherwise still run and
            // re-arm their own timer on a connection that is already gone,
            // leaving a deadline `handle_timeout` will never service because
            // it returns early once closed. A caller that polls the deadline
            // without checking `is_closed` first would then spin.
            if self.state == State::Closed {
                break;
            }

            match timer {
                Timer::Retransmit => self.handle_retransmit_timeout(now),
                Timer::AckDelay => self.handle_ack_delay_timeout(now),
                Timer::Idle => self.handle_idle_timeout(),
                Timer::KeepAlive => self.handle_keep_alive_timeout(now),
            }
        }
    }

    /// Retrieve the next event for the application layer.
    ///
    /// Returns `None` when there are no more events.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.pending_events.pop_front()
    }

    /// Initiate a graceful close of the connection.
    pub fn close(&mut self) {
        if self.state != State::Closed {
            self.state = State::Closed;
            self.timers.clear(Timer::Retransmit);
            self.timers.clear(Timer::AckDelay);
            self.timers.clear(Timer::Idle);
            self.timers.clear(Timer::KeepAlive);
            self.pending_events.push_back(Event::ConnectionClosed);
        }
    }

    /// Whether the connection is fully established and ready for data.
    pub fn is_established(&self) -> bool {
        self.state == State::Established
    }

    /// Whether the connection has been closed.
    pub fn is_closed(&self) -> bool {
        self.state == State::Closed
    }

    /// Current smoothed RTT estimate, if available.
    pub fn srtt(&self) -> Option<Duration> {
        self.rtt.srtt()
    }

    /// Current retransmission timeout.
    pub fn rto(&self) -> Duration {
        self.rtt.rto()
    }

    /// Negotiated MTU (maximum payload per packet), if handshake is complete.
    pub fn mtu(&self) -> Option<u16> {
        self.params.as_ref().map(|p| p.mtu)
    }

    /// The ACK delay timeout to use right now.
    ///
    /// [MS-RDPEUDP2] 3.1.5.2 gives the receiver's assumed default as "half
    /// the round trip time", with no floor or ceiling of its own for the
    /// RDP-UDP2 wire format. [MS-RDPEUDP] 3.1.6.3 gives the VERSION_2 analog
    /// for the v1 wire format a concrete shape, "50 ms or half the RTT,
    /// whichever is longer, up to a maximum of 200 ms"; that formula is used
    /// here as the closest available spec anchor for what RDPEUDP2's own
    /// silence on the floor and ceiling should mean, not as a v2-spec
    /// requirement in its own right.
    ///
    /// Falls back to the floor when no RTT sample exists yet. In practice
    /// this is only reachable when the handshake's own sample was skipped by
    /// Karn's algorithm (`sample_handshake_rtt`) because the handshake
    /// datagram was retransmitted.
    fn ack_delay_timeout(&self) -> Duration {
        const FLOOR: Duration = Duration::from_millis(50);
        const CEILING: Duration = Duration::from_millis(200);

        match self.rtt.srtt() {
            Some(srtt) => (srtt / 2).clamp(FLOOR, CEILING),
            None => FLOOR,
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Handshake: SYN generation
    // ════════════════════════════════════════════════════════════════

    /// Build and enqueue the client SYN datagram.
    fn enqueue_syn(&mut self, now: MonotonicInstant) {
        let datagram = V1Datagram {
            header: FecHeader {
                sn_source_ack: 0xFFFF_FFFF, // No prior packet to ACK
                receive_window_size: 1u16 << u16::from(self.config.log_window_size),
                flags: V1Flags::SYN | V1Flags::SYNEX,
            },
            ack_vector: None,
            ack_of_acks: None,
            syn_data: Some(SynDataPayload {
                initial_sequence_number: self.config.initial_sequence_number,
                upstream_mtu: self.config.upstream_mtu,
                downstream_mtu: self.config.downstream_mtu,
            }),
            correlation_id: None,
            syn_data_ex: Some(SynDataExPayload {
                syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
                // Version 3 is what selects the MS-RDPEUDP2 data transfer
                // (1.3.2.2). Version 2 is the same v1 data transfer with
                // shorter timers, so advertising it and then speaking v2
                // framing leaves the peer unable to parse anything.
                udp_ver: UdpVersion::V3,
                // 2.2.2.9: mandatory with version 3 in a client SYN.
                // `connect` refuses to build a connection without it.
                cookie_hash: self.config.cookie_hash,
            }),
        };

        if !self.enqueue_handshake(&datagram) {
            return;
        }
        self.handshake_sent_at = Some(now);

        self.timers.set(Timer::Retransmit, now + self.rtt.rto());
        self.timers.set(Timer::Idle, now + self.config.idle_timeout);
    }

    /// Build and enqueue the server SYN+ACK datagram.
    fn enqueue_syn_ack(&mut self, remote_isn: u32, now: MonotonicInstant) {
        let datagram = V1Datagram {
            header: FecHeader {
                sn_source_ack: remote_isn,
                receive_window_size: 1u16 << u16::from(self.config.log_window_size),
                flags: V1Flags::SYN | V1Flags::ACK | V1Flags::SYNEX,
            },
            // A SYN+ACK acknowledges the client's SYN through snSourceAck
            // alone (3.1.5.1.3); no ACK vector goes on the wire.
            ack_vector: None,
            ack_of_acks: None,
            syn_data: Some(SynDataPayload {
                initial_sequence_number: self.config.initial_sequence_number,
                upstream_mtu: self.config.upstream_mtu,
                downstream_mtu: self.config.downstream_mtu,
            }),
            correlation_id: None,
            syn_data_ex: Some(SynDataExPayload {
                syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
                udp_ver: UdpVersion::V3,
                // 2.2.2.9 puts the hash in the client's SYN and nowhere else:
                // "It MUST NOT be present in any other case."
                cookie_hash: None,
            }),
        };

        if !self.enqueue_handshake(&datagram) {
            return;
        }
        self.handshake_sent_at = Some(now);

        self.timers.set(Timer::Retransmit, now + self.rtt.rto());
    }

    /// Queue a handshake datagram and keep a copy for retransmission.
    ///
    /// These datagrams have a fixed shape, so the encode cannot fail in
    /// practice. If it somehow does there is no handshake to be had, and
    /// silently dropping the packet would leave the caller waiting on a
    /// connection that will never answer.
    ///
    /// Returns whether the datagram was queued. Callers must not arm timers
    /// when it was not: `close` has just cleared them, and re-arming leaves
    /// a deadline that `handle_timeout` returns early from without ever
    /// clearing.
    #[must_use]
    fn enqueue_handshake(&mut self, datagram: &V1Datagram) -> bool {
        let Ok(bytes) = encode_vec(datagram) else {
            self.close();
            return false;
        };

        self.pending_transmits.push_back(Transmit {
            contents: bytes.clone(),
        });
        self.handshake_datagram = Some(bytes);
        self.handshake_retransmits = 0;

        true
    }

    /// Put the last handshake datagram back on the wire because the peer
    /// repeated the one before it, so ours went missing.
    ///
    /// Not counted against `HANDSHAKE_RETRANSMIT_LIMIT`: that limit counts
    /// datagrams sent "without a response" (3.1.5.4.1), and this one is a
    /// response.
    fn resend_handshake_datagram(&mut self) {
        if let Some(datagram) = self.handshake_datagram.as_ref() {
            self.pending_transmits.push_back(Transmit {
                contents: datagram.clone(),
            });
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Handshake: receiving
    // ════════════════════════════════════════════════════════════════

    /// Client receives SYN+ACK from server.
    fn handle_syn_ack(&mut self, wire: &[u8], now: MonotonicInstant) -> Result<(), RdpeudpError> {
        let datagram: V1Datagram = decode(wire).map_err(RdpeudpError::decode)?;

        // Must have SYN + ACK flags
        if !datagram.header.flags.contains(V1Flags::SYN | V1Flags::ACK) {
            return Err(RdpeudpError::invalid_packet(
                "handle SYN+ACK",
                "expected SYN+ACK during handshake",
            ));
        }

        let syn_data = datagram
            .syn_data
            .as_ref()
            .ok_or_else(|| RdpeudpError::invalid_packet("handle SYN+ACK", "SYN+ACK missing SynData"))?;

        let syn_data_ex = datagram
            .syn_data_ex
            .as_ref()
            .ok_or_else(|| RdpeudpError::invalid_packet("handle SYN+ACK", "SYN+ACK missing SynDataEx"))?;

        // 3.1.5.1.1: the version in the SYN+ACK is "the highest version
        // supported by both endpoints", and per 3.1.5.1.3 that is the version
        // both MUST then use. Anything below 3 means the server settled on the
        // MS-RDPEUDP data transfer, which this crate does not speak.
        if !syn_data_ex.udp_ver.uses_v2_wire_format() {
            return Err(RdpeudpError::invalid_packet(
                "handle SYN+ACK",
                "server settled on a protocol version below 3, whose data transfer is MS-RDPEUDP rather than MS-RDPEUDP2",
            ));
        }

        let local_isn = self.config.initial_sequence_number;
        let remote_isn = syn_data.initial_sequence_number;

        let mtu = self
            .config
            .upstream_mtu
            .min(self.config.downstream_mtu)
            .min(syn_data.upstream_mtu)
            .min(syn_data.downstream_mtu);

        self.params = Some(NegotiatedParams {
            local_isn,
            remote_isn,
            mtu,
            log_window_size: self.config.log_window_size,
        });

        // Sample before enqueuing the final ACK. `enqueue_final_ack` calls
        // `enqueue_handshake`, which resets `handshake_retransmits` to 0 for
        // the new datagram; doing that first would erase whether the SYN
        // this SYN+ACK answers was itself retransmitted, defeating the
        // Karn's-algorithm check `sample_handshake_rtt` relies on.
        self.sample_handshake_rtt(now);

        // Send final ACK to complete handshake
        self.enqueue_final_ack(remote_isn);

        // Transition to established
        self.transition_to_established(now);

        Ok(())
    }

    /// Server receives the client's final ACK.
    fn handle_final_ack(&mut self, wire: &[u8], now: MonotonicInstant) -> Result<(), RdpeudpError> {
        let datagram: V1Datagram = decode(wire).map_err(RdpeudpError::decode)?;

        // The client repeats its SYN when our SYN+ACK goes missing. Answer it
        // again rather than reading it as a protocol violation.
        if datagram.header.flags.contains(V1Flags::SYN) {
            self.resend_handshake_datagram();
            return Ok(());
        }

        if !datagram.header.flags.contains(V1Flags::ACK) {
            return Err(RdpeudpError::invalid_packet(
                "handle final ACK",
                "expected ACK during handshake",
            ));
        }

        self.sample_handshake_rtt(now);

        self.transition_to_established(now);
        Ok(())
    }

    /// Build and enqueue the client's final ACK.
    fn enqueue_final_ack(&mut self, remote_isn: u32) {
        let datagram = V1Datagram {
            header: FecHeader {
                sn_source_ack: remote_isn,
                receive_window_size: 1u16 << u16::from(self.config.log_window_size),
                flags: V1Flags::ACK,
            },
            ack_vector: Some(V1AckVectorHeader {
                elements: vec![V1AckVectorElement {
                    state: VectorElementState::DatagramReceived,
                    length: 1,
                }],
            }),
            ack_of_acks: None,
            syn_data: None,
            correlation_id: None,
            syn_data_ex: None,
        };

        let _queued = self.enqueue_handshake(&datagram);
    }

    /// Sample the handshake round trip as the connection's first RTT
    /// estimate, from whichever handshake datagram we last sent and are
    /// now getting the answer to.
    ///
    /// Skipped if that datagram was retransmitted: Karn's algorithm (RFC
    /// 6298 Section 3) forbids RTT samples from retransmitted segments,
    /// since it is ambiguous which transmission this answer corresponds to.
    fn sample_handshake_rtt(&mut self, now: MonotonicInstant) {
        if self.handshake_retransmits != 0 {
            return;
        }
        if let Some(sent_at) = self.handshake_sent_at {
            self.rtt.update(now.duration_since(sent_at));
        }
    }

    // ════════════════════════════════════════════════════════════════
    // State transitions
    // ════════════════════════════════════════════════════════════════

    /// Transition from handshake to established state.
    ///
    /// Initializes all data transfer components (windows, timers).
    fn transition_to_established(&mut self, now: MonotonicInstant) {
        self.state = State::Established;
        self.timers.clear(Timer::Retransmit);

        // The client keeps its final ACK: nothing acknowledges that ACK, so
        // the only sign it was lost is the server repeating its SYN+ACK. The
        // server, having just received that ACK, has nothing left to repeat.
        if self.side == Side::Server {
            self.handshake_datagram = None;
        }

        let params = self
            .params
            .as_ref()
            .expect("params must be set before transitioning to established");

        // Data sequence numbers start at ISN + 1
        let local_initial_data_seq = u64::from(params.local_isn) + 1;
        let remote_initial_data_seq = u64::from(params.remote_isn) + 1;

        // Channel sequence numbers start at 1
        let initial_channel_seq = 1u64;

        self.send_window = Some(SendWindow::new(
            local_initial_data_seq,
            initial_channel_seq,
            params.log_window_size,
        ));

        self.recv_window = Some(RecvWindow::new(
            remote_initial_data_seq,
            initial_channel_seq,
            params.log_window_size,
        ));

        self.timers.set(Timer::Idle, now + self.config.idle_timeout);
        self.timers.set(Timer::KeepAlive, now + self.config.keep_alive_interval);

        self.pending_events.push_back(Event::Connected);
    }

    // ════════════════════════════════════════════════════════════════
    // V2 packet handling (established state)
    // ════════════════════════════════════════════════════════════════

    /// Process a v2 wire-format packet (with prefix byte framing).
    fn handle_v2_packet(&mut self, wire: &mut [u8], now: MonotonicInstant) -> Result<(), RdpeudpError> {
        let (prefix, packet_bytes) = decode_with_prefix(wire).map_err(RdpeudpError::prefix)?;

        let packet: V2Packet = decode(packet_bytes).map_err(RdpeudpError::decode)?;

        // Process ACK payload (cumulative acknowledgment)
        if let Some(ref ack) = packet.ack {
            self.process_ack(ack, &packet.header, now);
        }

        // Process AckVector (selective acknowledgment with gaps)
        if let Some(ref ack_vector) = packet.ack_vector {
            self.process_ack_vector(ack_vector, &packet.header, now);
        }

        // Process AckOfAcks (advance recv window base)
        if let Some(ref aoa) = packet.ack_of_acks {
            self.process_ack_of_acks(aoa);
        }

        // Process data payload
        if let (Some(dh), Some(db)) = (&packet.data_header, &packet.data_body) {
            if prefix.is_dummy() {
                // [MS-RDPEUDP2] 3.1.1.1.5: a dummy packet "is treated as a
                // normal RDP-UDP2 packet by the UDP transport", so it still
                // occupies its sequence number and gets acknowledged, but
                // "the contents MUST be ignored by higher layers using the
                // UDP transport".
                self.process_dummy_data(dh, now);
            } else {
                self.process_data(dh, db, now);
            }
        }

        Ok(())
    }

    /// Process a cumulative ACK payload.
    fn process_ack(&mut self, ack: &AckPayload, _header: &V2Header, now: MonotonicInstant) {
        let send_window = match self.send_window.as_mut() {
            Some(sw) => sw,
            None => return,
        };

        // Update remote timestamp reference from the received timestamp
        self.remote_timestamp_ref = seq::reconstruct_timestamp(ack.received_ts, self.remote_timestamp_ref);

        // Reconstruct the full 64-bit DataSeqNum from the 16-bit wire value
        let reference = send_window.next_data_seq().saturating_sub(1);
        let acked_seq = seq::reconstruct_seq(ack.seq_num, reference);

        // A peer cannot legitimately acknowledge a DataSeqNum that was never
        // assigned. Accepting one anyway would let `mark_received_through`
        // drain every currently outstanding entry as if it had been
        // delivered, discarding real in-flight application data on a single
        // malformed or hostile ACK.
        if acked_seq >= send_window.next_data_seq() {
            return;
        }

        // Time the acknowledged packet before acknowledging it. Resolving an
        // entry drains it off the front of the window, so looking it up
        // afterwards finds nothing and no RTT sample is ever taken: the
        // estimator stays empty and the RTO sits at its initial value for
        // the life of the connection.
        //
        // Karn's algorithm: only packets that went out once can be timed.
        let elapsed = send_window
            .get_by_data_seq(acked_seq)
            .filter(|entry| entry.transmit_count == 1)
            .map(|entry| now.duration_since(entry.sent_at));

        // The ACK seq_num is the highest sequentially received DataSeqNum, so
        // everything at or below it is acknowledged.
        let newly_acked_bytes = send_window.mark_received_through(acked_seq);

        if let Some(elapsed) = elapsed {
            let ack_gap = Duration::from_millis(u64::from(ack.send_ack_time_gap));
            if let Some(rtt_sample) = elapsed.checked_sub(ack_gap) {
                self.rtt.update(rtt_sample);
            }
        }

        // Congestion window growth
        if newly_acked_bytes > 0 {
            self.congestion.on_ack(newly_acked_bytes);
        }

        // Run loss detection after processing the ACK
        self.run_loss_detection(now);

        // New data acknowledged retires the packet the timer's deadline was
        // set for, so a still-outstanding later packet would otherwise be
        // measured against a deadline meant for something else and can be
        // declared lost within milliseconds instead of a full RTO. Clearing
        // only on real progress, never on a duplicate ACK, is what keeps a
        // peer from postponing loss detection indefinitely by repeating one.
        if newly_acked_bytes > 0 {
            self.timers.clear(Timer::Retransmit);
        }
        self.update_retransmit_timer(now);

        // 3.1.5.3: stop advertising once the peer acknowledges past it.
        self.retire_ack_of_acks(acked_seq);
    }

    /// Process a selective ACK vector.
    fn process_ack_vector(&mut self, ack_vector: &AckVectorPayload, _header: &V2Header, now: MonotonicInstant) {
        let send_window = match self.send_window.as_mut() {
            Some(sw) => sw,
            None => return,
        };

        let reference = send_window.next_data_seq().saturating_sub(1);
        let base = seq::reconstruct_seq(ack_vector.base_seq_num, reference);

        // Walk the ack vector entries and mark packets received/not-received
        let mut current_seq = base;
        let mut newly_acked_bytes: u64 = 0;

        // Timing for the highest packet this vector acknowledges, taken as we
        // go because resolving an entry drains it out of the window. Karn's
        // algorithm: only packets that went out once can be timed.
        let mut elapsed = None;

        for entry in &ack_vector.entries {
            match entry {
                AckVectorEntry::RunLength { received, length } => {
                    for _ in 0..u64::from(*length) {
                        if *received {
                            if let Some(sample) = send_window
                                .get_by_data_seq(current_seq)
                                .filter(|entry| entry.transmit_count == 1)
                                .map(|entry| now.duration_since(entry.sent_at))
                            {
                                elapsed = Some(sample);
                            }

                            if let Some(size) = send_window.mark_received(current_seq) {
                                newly_acked_bytes += u64::try_from(size).expect("packet size fits in u64");
                            }
                        }
                        current_seq += 1;
                    }
                }
                AckVectorEntry::StateMap { bitmap } => {
                    // [MS-RDPEUDP2] 2.2.1.2.6 state-map mode: the most
                    // significant bit is the mode marker and the remaining
                    // seven carry one sequence number each, so a state-map byte
                    // covers seven, not eight. Advancing by eight desynchronised
                    // the cursor from the peer's vector for every entry after
                    // the first state map.
                    for bit_pos in 0..7u32 {
                        let received = (bitmap >> bit_pos) & 1 == 1;
                        if received {
                            if let Some(sample) = send_window
                                .get_by_data_seq(current_seq)
                                .filter(|entry| entry.transmit_count == 1)
                                .map(|entry| now.duration_since(entry.sent_at))
                            {
                                elapsed = Some(sample);
                            }

                            if let Some(size) = send_window.mark_received(current_seq) {
                                newly_acked_bytes += u64::try_from(size).expect("packet size fits in u64");
                            }
                        }
                        current_seq += 1;
                    }
                }
            }
        }

        // RTT estimation from ACKVEC timing
        if let (Some(elapsed), Some(gap_ms)) = (elapsed, ack_vector.send_ack_time_gap_ms) {
            let ack_gap = Duration::from_millis(u64::from(gap_ms));
            if let Some(rtt_sample) = elapsed.checked_sub(ack_gap) {
                self.rtt.update(rtt_sample);
            }
        }

        if newly_acked_bytes > 0 {
            self.congestion.on_ack(newly_acked_bytes);
        }

        self.run_loss_detection(now);

        // See the matching comment in `process_ack`: restart on real
        // progress only, so a duplicate ACK cannot postpone the deadline.
        if newly_acked_bytes > 0 {
            self.timers.clear(Timer::Retransmit);
        }
        self.update_retransmit_timer(now);

        // 3.1.5.3 also retires it on "an acknowledgment vector with its lowest
        // sequence number still missing that is higher than the AckOfAck
        // sequence number", which is this vector's base.
        self.retire_ack_of_acks(base);
    }

    /// Process an AckOfAcks payload: advance our recv window base.
    fn process_ack_of_acks(&mut self, aoa: &AckOfAcksPayload) {
        let recv_window = match self.recv_window.as_mut() {
            Some(rw) => rw,
            None => return,
        };

        let reference = recv_window.highest_seq();
        let new_base = seq::reconstruct_seq(aoa.ack_of_acks_seq_num, reference);
        recv_window.advance_base(new_base);
    }

    /// Account for a dummy packet without handing anything to the
    /// application.
    ///
    /// [MS-RDPEUDP2] 3.1.1.1.5 keeps the transport's view of a dummy packet
    /// ordinary: it fills its slot in the receive window and is acknowledged,
    /// which is what lets the sender see it arrive. Only the contents are
    /// off limits.
    fn process_dummy_data(&mut self, dh: &DataHeader, now: MonotonicInstant) {
        let Some(recv_window) = self.recv_window.as_mut() else {
            return;
        };

        let reference = recv_window.highest_seq();
        let data_seq = seq::reconstruct_seq(dh.data_seq_num, reference);

        if !recv_window.receive_without_payload(data_seq) {
            return;
        }

        self.ack_pending = true;
        if !self.timers.is_set(Timer::AckDelay) {
            self.timers.set(Timer::AckDelay, now + self.ack_delay_timeout());
            self.ack_delay_started_at = Some(now);
        }
    }

    fn process_data(&mut self, dh: &DataHeader, db: &DataBody, now: MonotonicInstant) {
        let recv_window = match self.recv_window.as_mut() {
            Some(rw) => rw,
            None => return,
        };

        // Reconstruct full sequence numbers from 16-bit wire values
        let reference_data = recv_window.highest_seq();
        let data_seq = seq::reconstruct_seq(dh.data_seq_num, reference_data);

        let reference_channel = recv_window.next_channel_seq();
        let channel_seq = seq::reconstruct_seq(db.channel_seq_num, reference_channel);

        // Record the packet in the receive window
        let is_new = recv_window.receive(data_seq, channel_seq, db.data.clone());

        if is_new {
            // Drain ordered data for application delivery
            let delivered = recv_window.drain_ordered();
            for chunk in delivered {
                self.pending_events.push_back(Event::DataReceived(chunk));
            }

            // Schedule ACK (either piggybacked on next data or standalone)
            self.ack_pending = true;
            if !self.timers.is_set(Timer::AckDelay) {
                self.timers.set(Timer::AckDelay, now + self.ack_delay_timeout());
                self.ack_delay_started_at = Some(now);
            }
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Loss detection and retransmission
    // ════════════════════════════════════════════════════════════════

    /// Run the loss detector on the current send window state.
    fn run_loss_detection(&mut self, now: MonotonicInstant) {
        let send_window = match self.send_window.as_mut() {
            Some(sw) => sw,
            None => return,
        };

        let highest_acked = send_window.highest_acked_data_seq();
        let rto = self.rtt.rto();

        let lost_seqs = self.loss_detector.detect(send_window, highest_acked, rto, now);

        for data_seq in lost_seqs {
            self.declare_lost(data_seq);
        }
    }

    /// Move one packet out of the send window and onto the retransmit queue.
    fn declare_lost(&mut self, data_seq: u64) {
        let Some(send_window) = self.send_window.as_mut() else {
            return;
        };

        // Read before `mark_lost` drops the entry: this bounds the recovery
        // epoch, so everything already in flight counts as one loss event.
        let largest_sent = send_window.next_data_seq().saturating_sub(1);

        let Some(info) = send_window.mark_lost(data_seq) else {
            return;
        };

        // Losing the packet moved the front of the window past it, and the
        // retransmission will carry a new DataSeqNum, so this one is never
        // going to be acknowledged. Tell the peer where we have got to, or its
        // receive window will sit on the hole forever (3.1.5.3).
        let lowest_unacknowledged = send_window.lowest_unacknowledged();

        self.congestion.on_loss(data_seq, largest_sent);
        self.reliability.enqueue(info.channel_seq, info.data);
        self.pending_ack_of_acks = Some(lowest_unacknowledged);
    }

    /// Stop advertising the AckOfAcks once the peer has moved past it.
    ///
    /// [MS-RDPEUDP2] 3.1.5.3: the sender "should stop sending this payload
    /// when either it receives an acknowledgment that has a sequence number
    /// that is higher than the AckOfAck sequence number, or it receives an
    /// acknowledgment vector with its lowest sequence number still missing
    /// that is higher than the AckOfAck sequence number".
    fn retire_ack_of_acks(&mut self, peer_reported_seq: u64) {
        if self
            .pending_ack_of_acks
            .is_some_and(|advertised| peer_reported_seq > advertised)
        {
            self.pending_ack_of_acks = None;
        }
    }

    /// Declare the oldest unacknowledged packet lost because the retransmit
    /// timer expired on it.
    ///
    /// The timer firing is itself the loss signal: [MS-RDPEUDP] 3.1.6.1 arms
    /// it for a datagram that has gone unacknowledged for an RTO, and 3.1.1.5
    /// leaves the choice of retransmit scheme to the implementation. Without
    /// this, a packet lost with fewer than `DEFAULT_REORDER_THRESHOLD` later
    /// packets behind it is never retransmitted at all, because the time
    /// threshold in `LossDetector` is computed from an RTO that
    /// `RttEstimator::on_timeout` has already doubled and so always sits
    /// ahead of the elapsed time.
    fn declare_oldest_pending_lost(&mut self) {
        let Some(send_window) = self.send_window.as_ref() else {
            return;
        };

        let Some(oldest) = send_window.pending_entries().map(|entry| entry.data_seq).min() else {
            return;
        };

        self.declare_lost(oldest);
    }

    /// Update the retransmit timer based on send window state.
    fn update_retransmit_timer(&mut self, now: MonotonicInstant) {
        let has_pending = match self.send_window.as_ref() {
            Some(send_window) => send_window.pending_entries().next().is_some(),
            None => return,
        };

        // The retransmit queue counts too. A packet declared lost leaves the
        // send window immediately but may sit in the queue while the
        // congestion window is full, and with no timer armed nothing would
        // come back to it if the peer had also gone quiet.
        if has_pending || self.reliability.has_pending() {
            // Keep the timer running
            if !self.timers.is_set(Timer::Retransmit) {
                self.timers.set(Timer::Retransmit, now + self.rtt.rto());
            }
        } else {
            // Nothing outstanding: clear the timer
            self.timers.clear(Timer::Retransmit);
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Transmit generation
    // ════════════════════════════════════════════════════════════════

    /// Try to build and send a retransmission from the reliability queue.
    fn try_retransmit(&mut self, now: MonotonicInstant) -> Option<Transmit> {
        if !self.reliability.has_pending() {
            return None;
        }

        let send_window = self.send_window.as_mut()?;

        // Check congestion window allows sending
        if send_window.bytes_in_flight() >= self.congestion.window() {
            return None;
        }

        if !send_window.has_capacity() {
            return None;
        }

        let entry = self.reliability.dequeue()?;

        // Create a new DataSeqNum for the retransmit but preserve ChannelSeqNum
        let new_data_seq = send_window.push_retransmit(entry.channel_seq, entry.data.clone(), now)?;

        let transmit = self.build_data_packet(new_data_seq, entry.channel_seq, entry.data, now);

        if transmit.is_some() {
            // Reset retransmit timer
            self.timers.set(Timer::Retransmit, now + self.rtt.rto());
        }

        transmit
    }

    /// Try to build and send new data from the send buffer.
    fn try_send_new_data(&mut self, now: MonotonicInstant) -> Option<Transmit> {
        if self.send_buffer.is_empty() {
            return None;
        }

        let send_window = self.send_window.as_mut()?;

        // Check congestion window
        if send_window.bytes_in_flight() >= self.congestion.window() {
            return None;
        }

        if !send_window.has_capacity() {
            return None;
        }

        let data = self.send_buffer.pop_front()?;

        // Push into send window (assigns DataSeqNum and ChannelSeqNum)
        let (data_seq, channel_seq) = send_window.push(data.clone(), now)?;

        let transmit = self.build_data_packet(data_seq, channel_seq, data, now);

        if transmit.is_some() {
            // Set retransmit timer if not already running
            if !self.timers.is_set(Timer::Retransmit) {
                self.timers.set(Timer::Retransmit, now + self.rtt.rto());
            }
        }

        transmit
    }

    /// Build a V2 data packet with optional ACK piggybacking.
    fn build_data_packet(
        &mut self,
        data_seq: u64,
        channel_seq: u64,
        data: Vec<u8>,
        now: MonotonicInstant,
    ) -> Option<Transmit> {
        let log_window_size = self.params.as_ref()?.log_window_size;

        let mut flags = V2Flags::DATA;

        // Piggyback an acknowledgment if one is pending
        let mut acknowledged = false;
        let (ack, ack_vector) = if self.ack_pending {
            match self.build_acknowledgement(now) {
                Some(acknowledgement) => {
                    acknowledged = true;
                    flags |= acknowledgement.flag;
                    (acknowledgement.ack, acknowledgement.ack_vector)
                }
                None => (None, None),
            }
        } else {
            (None, None)
        };

        // AckOfAcks if pending
        let ack_of_acks = self.pending_ack_of_acks.map(|seq| AckOfAcksPayload {
            ack_of_acks_seq_num: seq::truncate_seq(seq),
        });
        if ack_of_acks.is_some() {
            flags |= V2Flags::AOA;
        }

        let packet = V2Packet {
            header: V2Header { flags, log_window_size },
            ack,
            overhead_size: None,
            delay_ack_info: None,
            ack_of_acks,
            data_header: Some(DataHeader {
                data_seq_num: seq::truncate_seq(data_seq),
            }),
            ack_vector,
            data_body: Some(DataBody {
                channel_seq_num: seq::truncate_seq(channel_seq),
                data,
            }),
        };

        let transmit = self.encode_v2_packet(&packet)?;

        // Retire what the packet carried only now that it exists. Clearing
        // any of this before the encode drops an acknowledgment that was
        // never sent, and the peer has no way to learn it was owed one.
        if acknowledged {
            self.ack_pending = false;
            self.timers.clear(Timer::AckDelay);
            self.commit_acknowledgement();
        }

        Some(transmit)
    }

    /// Build a standalone ACK (no data).
    fn build_standalone_ack(&mut self, now: MonotonicInstant) -> Option<Transmit> {
        let log_window_size = self.params.as_ref()?.log_window_size;

        let acknowledgement = self.build_acknowledgement(now)?;
        let mut flags = acknowledgement.flag;
        let (ack, ack_vector) = (acknowledgement.ack, acknowledgement.ack_vector);

        let ack_of_acks = self.pending_ack_of_acks.map(|seq| AckOfAcksPayload {
            ack_of_acks_seq_num: seq::truncate_seq(seq),
        });
        if ack_of_acks.is_some() {
            flags |= V2Flags::AOA;
        }

        let packet = V2Packet {
            header: V2Header { flags, log_window_size },
            ack,
            overhead_size: None,
            delay_ack_info: None,
            ack_of_acks,
            data_header: None,
            ack_vector,
            data_body: None,
        };

        let transmit = self.encode_v2_packet(&packet)?;

        // Same ordering as the data path: the window only moves past packets
        // whose acknowledgment actually made it into a packet.
        self.commit_acknowledgement();

        Some(transmit)
    }

    /// Build the cumulative ACK payload from the recv window state.
    /// Build the acknowledgment to attach to an outgoing packet.
    ///
    /// A cumulative ACK claims every packet through its sequence number, so
    /// it is only true when the window has no holes. [MS-RDPEUDP2] 3.1.1.2.2
    /// keeps the vector form for the other case. Sending the cumulative form
    /// over a hole tells the sender to stop retransmitting a packet that
    /// never arrived, and the receiver never asks for it again.
    fn build_acknowledgement(&self, now: MonotonicInstant) -> Option<Acknowledgement> {
        let recv_window = self.recv_window.as_ref()?;

        let acknowledgement = if recv_window.has_gaps() {
            Acknowledgement {
                flag: V2Flags::ACKVEC,
                ack: None,
                ack_vector: Some(self.build_ack_vector_payload(recv_window, now)),
            }
        } else {
            Acknowledgement {
                flag: V2Flags::ACK,
                ack: Some(self.build_ack_payload(recv_window, now)),
                ack_vector: None,
            }
        };

        Some(acknowledgement)
    }

    /// Retire the receive-window slots covered by an acknowledgment that is
    /// now encoded and on its way out.
    ///
    /// Separate from building it so a failed encode cannot advance the
    /// window past packets whose acknowledgment never reached the wire.
    fn commit_acknowledgement(&mut self) {
        if let Some(recv_window) = self.recv_window.as_mut() {
            recv_window.release_acknowledged();
        }
        self.ack_delay_started_at = None;
    }

    /// Convert an instant to the wire timestamp's 4us units.
    ///
    /// `MonotonicInstant` is millisecond resolution, an exact multiple of
    /// the 4us unit, so this loses no precision the crate's time model has
    /// in the first place.
    fn timestamp_units(instant: MonotonicInstant) -> u64 {
        seq::us_to_timestamp_units(instant.as_millis() * 1_000)
    }

    fn build_ack_payload(&self, recv_window: &RecvWindow, now: MonotonicInstant) -> AckPayload {
        // seq_num = highest sequentially received DataSeqNum
        // For cumulative ACK, this is base_seq + count of contiguous received - 1,
        // but recv_window.highest_seq() gives us the overall highest.
        // For a true cumulative ACK without gaps, highest_seq is correct.
        // When gaps exist, we use ACKVEC instead.
        let cumulative_seq = recv_window.highest_seq();
        let received_at = self.ack_delay_started_at.unwrap_or(now);

        AckPayload {
            seq_num: seq::truncate_seq(cumulative_seq),
            received_ts: seq::truncate_timestamp(Self::timestamp_units(received_at)),
            // MS-RDPEUDP2 2.2.1.2.1 gives this field the full 0..=255 range,
            // unlike the vector payload's equivalent below.
            send_ack_time_gap: u8::try_from(now.duration_since(received_at).as_millis()).unwrap_or(u8::MAX),
            delay_ack_time_scale: 0,
            delay_ack_time_additions: Vec::new(),
        }
    }

    /// Build the selective ACK vector payload from recv window state.
    ///
    /// `codedAckVecSize` is a 7-bit wire field, so at most
    /// [`ACK_VECTOR_MAX_ENTRIES`] entries fit. An alternating received /
    /// not-received pattern needs one entry per slot, which a configured
    /// window above that many slots can produce. Describing only a prefix of
    /// the window here, rather than failing the whole encode once the limit
    /// is hit, is what keeps that case making progress: a truncated vector is
    /// still an accurate description of everything up to where it stops, and
    /// the next acknowledgment picks up wherever the window has moved by
    /// then.
    fn build_ack_vector_payload(&self, recv_window: &RecvWindow, now: MonotonicInstant) -> AckVectorPayload {
        let received_at = self.ack_delay_started_at.unwrap_or(now);
        let ack_vec = recv_window.ack_vector();
        let mut entries: Vec<AckVectorEntry> = Vec::new();

        // RunLength entries have a 6-bit length field (max 63).
        // Split longer runs into multiple entries.
        'runs: for (received, count) in ack_vec {
            let mut remaining = count;
            while remaining > 0 {
                if entries.len() >= ACK_VECTOR_MAX_ENTRIES {
                    break 'runs;
                }
                let chunk = u8::try_from(remaining.min(63)).expect("clamped to 6-bit range");
                entries.push(AckVectorEntry::RunLength {
                    received,
                    length: chunk,
                });
                remaining -= u64::from(chunk);
            }
        }

        AckVectorPayload {
            base_seq_num: seq::truncate_seq(recv_window.base_seq()),
            timestamp: Some(seq::truncate_timestamp(Self::timestamp_units(received_at))),
            // MS-RDPEUDP2 2.2.1.2.6 reserves 255 to mean "invalid, MUST NOT
            // be used", unlike the cumulative payload's equivalent above.
            send_ack_time_gap_ms: Some(
                u8::try_from(now.duration_since(received_at).as_millis())
                    .unwrap_or(254)
                    .min(254),
            ),
            entries,
        }
    }

    /// Encode a V2 packet with the PacketPrefixByte framing.
    fn encode_v2_packet(&mut self, packet: &V2Packet) -> Option<Transmit> {
        let packet_bytes = encode_vec(packet).ok()?;
        // We never originate dummy packets. They exist for a sender that wants
        // to probe or pad without giving the higher layer anything to read,
        // and this transport has no use for that.
        let is_dummy = false;

        self.wire_buf.clear();
        encode_with_prefix(&packet_bytes, is_dummy, &mut self.wire_buf).ok()?;

        Some(Transmit {
            contents: self.wire_buf.clone(),
        })
    }

    // ════════════════════════════════════════════════════════════════
    // Timer handlers
    // ════════════════════════════════════════════════════════════════

    /// Handle retransmit timer expiry.
    fn handle_retransmit_timeout(&mut self, now: MonotonicInstant) {
        self.timers.clear(Timer::Retransmit);

        if self.state != State::Established {
            self.rtt.on_timeout();
            self.retransmit_handshake(now);
            return;
        }

        // Before the backoff, which moves the threshold this would be
        // measured against out of reach. See `declare_oldest_pending_lost`.
        self.declare_oldest_pending_lost();

        // Apply exponential backoff
        self.rtt.on_timeout();

        // Reset the timer if there is still unfinished business
        self.update_retransmit_timer(now);
    }

    /// Resend the handshake datagram the peer has not answered, or close
    /// once it has had enough chances (3.1.5.4.1).
    fn retransmit_handshake(&mut self, now: MonotonicInstant) {
        if self.handshake_datagram.is_none() {
            return;
        }

        if self.handshake_retransmits >= HANDSHAKE_RETRANSMIT_LIMIT {
            self.close();
            return;
        }

        self.handshake_retransmits += 1;
        self.resend_handshake_datagram();

        // 3.1.6.1 wants the timer to keep firing at no less than the same
        // interval; `on_timeout` above has already backed the RTO off.
        self.timers.set(Timer::Retransmit, now + self.rtt.rto());
    }

    /// Handle ACK delay timer expiry: send a standalone ACK.
    fn handle_ack_delay_timeout(&mut self, _now: MonotonicInstant) {
        self.timers.clear(Timer::AckDelay);
        self.ack_pending = true;
    }

    /// Handle idle timeout: close the connection.
    fn handle_idle_timeout(&mut self) {
        self.close();
    }

    /// Handle keep-alive timer: enqueue a keep-alive probe.
    fn handle_keep_alive_timeout(&mut self, now: MonotonicInstant) {
        self.timers.clear(Timer::KeepAlive);

        // Send a standalone ACK as a keep-alive probe
        self.ack_pending = true;

        self.timers.set(Timer::KeepAlive, now + self.config.keep_alive_interval);
    }
}

impl core::fmt::Debug for RdpeudpConnection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RdpeudpConnection")
            .field("side", &self.side)
            .field("state", &self.state)
            .field("rto", &self.rtt.rto())
            .field("srtt", &self.rtt.srtt())
            .field("pending_transmits", &self.pending_transmits.len())
            .field("pending_events", &self.pending_events.len())
            .field("send_buffer", &self.send_buffer.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worst data packet this connection can build still fits the MTU.
    ///
    /// `PIGGYBACK_RESERVE` is arithmetic over payload sizes that live in
    /// another module, so measure it rather than trust it. If any of those
    /// payloads grows, this fails instead of a peer silently receiving an
    /// over-length datagram.
    #[test]
    fn a_full_data_packet_fits_the_mtu() {
        let max_payload = RDPEUDP2_MTU - DATA_PACKET_OVERHEAD - PIGGYBACK_RESERVE;

        let packet = V2Packet {
            header: V2Header {
                flags: V2Flags::DATA | V2Flags::ACKVEC | V2Flags::AOA,
                log_window_size: 6,
            },
            ack: None,
            overhead_size: None,
            delay_ack_info: None,
            ack_of_acks: Some(AckOfAcksPayload {
                ack_of_acks_seq_num: 0xFFFF,
            }),
            data_header: Some(DataHeader { data_seq_num: 0xFFFF }),
            // The biggest vector the 7-bit size field can describe, with the
            // optional timestamp block present.
            ack_vector: Some(AckVectorPayload {
                base_seq_num: 0xFFFF,
                timestamp: Some(0x00FF_FFFF),
                send_ack_time_gap_ms: Some(0xFF),
                entries: vec![
                    AckVectorEntry::RunLength {
                        received: true,
                        length: 63,
                    };
                    127
                ],
            }),
            data_body: Some(DataBody {
                channel_seq_num: 0xFFFF,
                data: vec![0xAB; max_payload],
            }),
        };

        let encoded = encode_vec(&packet).expect("encode");

        let mut wire = Vec::new();
        encode_with_prefix(&encoded, false, &mut wire).expect("prefix");

        assert!(
            wire.len() <= RDPEUDP2_MTU,
            "a full data packet is {} bytes, over the {RDPEUDP2_MTU} byte MTU",
            wire.len()
        );
    }

    /// The reserve should not be wildly larger than it needs to be either.
    #[test]
    fn the_piggyback_reserve_is_not_wasteful() {
        let max_payload = RDPEUDP2_MTU - DATA_PACKET_OVERHEAD - PIGGYBACK_RESERVE;

        assert!(
            max_payload >= 1000,
            "only {max_payload} bytes of every packet are usable"
        );
    }

    /// An alternating received/not-received pattern needs one run-length
    /// entry per slot, which a window above 127 slots can produce more of
    /// than the 7-bit wire field allows. The vector must still respect the
    /// limit and still encode, rather than describing the whole window and
    /// failing the encode outright.
    #[test]
    fn build_ack_vector_payload_truncates_to_the_wire_limit() {
        let conn = RdpeudpConnection::new(Side::Client, test_config());

        let mut recv_window = RecvWindow::new(1, 1, 8); // 256 slots
        for seq in 1..256u64 {
            if seq % 2 == 1 {
                recv_window.receive_without_payload(seq);
            }
        }

        let payload = conn.build_ack_vector_payload(&recv_window, MonotonicInstant::from_millis(0));

        assert!(
            payload.entries.len() <= ACK_VECTOR_MAX_ENTRIES,
            "got {} entries, over the wire limit of {ACK_VECTOR_MAX_ENTRIES}",
            payload.entries.len()
        );
        encode_vec(&payload).expect("a truncated vector must still encode");
    }

    /// Both ACK builders must report the real receive-to-send gap, not a
    /// hardcoded zero: an always-zero gap makes the peer fold the entire
    /// delayed-ACK hold time into its RTT sample instead of only the network
    /// transit time.
    #[test]
    fn ack_builders_report_the_real_delay_gap_and_timestamp() {
        let mut conn = RdpeudpConnection::new(Side::Client, test_config());
        conn.ack_delay_started_at = Some(MonotonicInstant::from_millis(100));
        let now = MonotonicInstant::from_millis(140);

        let mut recv_window = RecvWindow::new(1, 1, 6);
        recv_window.receive_without_payload(1);

        let expected_ts =
            seq::truncate_timestamp(RdpeudpConnection::timestamp_units(MonotonicInstant::from_millis(100)));

        let ack = conn.build_ack_payload(&recv_window, now);
        assert_eq!(ack.send_ack_time_gap, 40);
        assert_eq!(ack.received_ts, expected_ts);

        let vector = conn.build_ack_vector_payload(&recv_window, now);
        assert_eq!(vector.send_ack_time_gap_ms, Some(40));
        assert_eq!(vector.timestamp, Some(expected_ts));
    }

    /// With nothing outstanding, `ack_delay_started_at` is `None` and the
    /// gap must read as zero rather than panicking or reporting stale state.
    #[test]
    fn ack_builders_report_a_zero_gap_with_nothing_pending() {
        let conn = RdpeudpConnection::new(Side::Client, test_config());
        let now = MonotonicInstant::from_millis(140);

        let mut recv_window = RecvWindow::new(1, 1, 6);
        recv_window.receive_without_payload(1);

        let ack = conn.build_ack_payload(&recv_window, now);
        assert_eq!(ack.send_ack_time_gap, 0);

        let vector = conn.build_ack_vector_payload(&recv_window, now);
        assert_eq!(vector.send_ack_time_gap_ms, Some(0));
    }

    fn test_config() -> ConnectionConfig {
        ConnectionConfig {
            cookie_hash: Some([0x5A; 32]),
            ..Default::default()
        }
    }

    #[test]
    fn ack_delay_timeout_floors_when_no_rtt_sample() {
        let conn = RdpeudpConnection::new(Side::Client, test_config());
        assert_eq!(conn.ack_delay_timeout(), Duration::from_millis(50));
    }

    #[test]
    fn ack_delay_timeout_floors_below_the_50ms_minimum() {
        let mut conn = RdpeudpConnection::new(Side::Client, test_config());
        conn.rtt.update(Duration::from_millis(60)); // half = 30ms, below the floor
        assert_eq!(conn.ack_delay_timeout(), Duration::from_millis(50));
    }

    #[test]
    fn ack_delay_timeout_tracks_half_the_srtt_between_the_bounds() {
        let mut conn = RdpeudpConnection::new(Side::Client, test_config());
        conn.rtt.update(Duration::from_millis(300)); // half = 150ms, within bounds
        assert_eq!(conn.ack_delay_timeout(), Duration::from_millis(150));
    }

    #[test]
    fn ack_delay_timeout_caps_above_the_200ms_maximum() {
        let mut conn = RdpeudpConnection::new(Side::Client, test_config());
        conn.rtt.update(Duration::from_secs(1)); // half = 500ms, above the cap
        assert_eq!(conn.ack_delay_timeout(), Duration::from_millis(200));
    }

    #[test]
    fn sample_handshake_rtt_updates_the_estimator() {
        let mut conn = RdpeudpConnection::new(Side::Client, test_config());
        let sent_at = MonotonicInstant::from_millis(1_000);
        conn.handshake_sent_at = Some(sent_at);

        conn.sample_handshake_rtt(sent_at + Duration::from_millis(40));

        assert_eq!(conn.rtt.srtt(), Some(Duration::from_millis(40)));
    }

    /// Karn's algorithm (RFC 6298 Section 3): a retransmitted handshake
    /// datagram makes it ambiguous which transmission an answer is for, so
    /// no sample should be taken.
    #[test]
    fn sample_handshake_rtt_skips_after_a_retransmit() {
        let mut conn = RdpeudpConnection::new(Side::Client, test_config());
        let sent_at = MonotonicInstant::from_millis(1_000);
        conn.handshake_sent_at = Some(sent_at);
        conn.handshake_retransmits = 1;

        conn.sample_handshake_rtt(sent_at + Duration::from_millis(40));

        assert_eq!(conn.rtt.srtt(), None);
    }
}
