#![doc = "Reliable RDP-UDP transport state machine."]
#![no_std]
#![forbid(unsafe_code)]
#![warn(clippy::std_instead_of_alloc)]
#![warn(clippy::std_instead_of_core)]

//! A no-I/O implementation of the reliable RDP-UDP data plane.
//!
//! The caller owns socket I/O and time. It passes monotonically increasing
//! [`Timestamp`] values to this state machine, sends each emitted datagram, and
//! feeds received datagrams back through [`ReliableUdp::handle_datagram`].
//! This deliberate split makes timers deterministic and keeps the crate usable
//! in Tokio and other runtimes.
//!
//! RDP-UDP version 3 is deliberately unavailable until its required
//! security-cookie binding and RDP-UDP2 data plane are complete.

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(test)]
extern crate std;

#[cfg(feature = "alloc")]
use alloc::collections::{BTreeMap, VecDeque};
#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};
use core::time::Duration;

/// The largest source payload accepted by RDP-UDP MTU negotiation.
pub const MAX_MTU: usize = 1232;
/// The smallest source payload accepted by RDP-UDP MTU negotiation.
pub const MIN_MTU: usize = 1132;
/// The largest complete RDP-UDP datagram accepted by this implementation.
///
/// This includes a maximum-size protocol data payload, the largest permitted
/// ACK vector, and the larger FEC payload header.
pub const MAX_DATAGRAM_SIZE: usize =
    MAX_MTU + 8 /* RDPUDP_FEC_HEADER */ + 2 /* uAckVectorSize */ + 2048 /* AckVector */
        + 2 /* AckVector padding */ + 12 /* RDPUDP_FEC_PAYLOAD_HEADER */;

#[cfg(feature = "alloc")]
const FLAG_SYN: u16 = 0x0001;
#[cfg(feature = "alloc")]
const FLAG_ACK: u16 = 0x0004;
#[cfg(feature = "alloc")]
const FLAG_DATA: u16 = 0x0008;
#[cfg(feature = "alloc")]
const FLAG_FEC: u16 = 0x0010;
#[cfg(feature = "alloc")]
const FLAG_CN: u16 = 0x0020;
#[cfg(feature = "alloc")]
const FLAG_CWR: u16 = 0x0040;
#[cfg(feature = "alloc")]
const FLAG_ACK_OF_ACKS: u16 = 0x0100;
#[cfg(feature = "alloc")]
const FLAG_SYN_LOSSY: u16 = 0x0200;
#[cfg(feature = "alloc")]
const FLAG_ACK_DELAYED: u16 = 0x0400;
#[cfg(feature = "alloc")]
const FLAG_CORRELATION_ID: u16 = 0x0800;
#[cfg(feature = "alloc")]
const FLAG_SYN_EX: u16 = 0x1000;
#[cfg(feature = "alloc")]
const SYN_EX_VERSION_INFO_VALID: u16 = 0x0001;
#[cfg(feature = "alloc")]
const COMMON_HEADER_SIZE: usize = 8;
#[cfg(feature = "alloc")]
const SYN_DATA_SIZE: usize = 8;
#[cfg(feature = "alloc")]
const CORRELATION_ID_PAYLOAD_SIZE: usize = 32;
#[cfg(feature = "alloc")]
const SOURCE_HEADER_SIZE: usize = 8;
#[cfg(feature = "alloc")]
const FEC_HEADER_SIZE: usize = 12;
#[cfg(feature = "alloc")]
const ACK_VECTOR_HEADER_SIZE: usize = 2;
const MAX_ACK_VECTOR_SIZE: usize = 2048;
#[cfg(feature = "alloc")]
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(16);
#[cfg(feature = "alloc")]
const PEER_TIMEOUT: Duration = Duration::from_secs(65);
#[cfg(feature = "alloc")]
const V3_HEADER_SIZE: usize = 2;
#[cfg(feature = "alloc")]
const V3_DATA_HEADER_SIZE: usize = 2;
#[cfg(feature = "alloc")]
const V3_CHANNEL_HEADER_SIZE: usize = 2;
#[cfg(feature = "alloc")]
const V3_ACK_SIZE: usize = 7;
#[cfg(feature = "alloc")]
const V3_WIRE_OVERHEAD: usize =
    1 /* PacketPrefixByte */ + V3_HEADER_SIZE + V3_DATA_HEADER_SIZE + V3_CHANNEL_HEADER_SIZE;
#[cfg(feature = "alloc")]
const V3_FLAG_ACK: u16 = 0x001;
#[cfg(feature = "alloc")]
const V3_FLAG_DATA: u16 = 0x004;
#[cfg(feature = "alloc")]
const V3_FLAG_ACKVEC: u16 = 0x008;
#[cfg(feature = "alloc")]
const V3_SUPPORTED_FLAGS: u16 = V3_FLAG_ACK | V3_FLAG_DATA;

/// A monotonic timestamp supplied by the embedding runtime.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(Duration);

impl Timestamp {
    /// Builds a timestamp from elapsed monotonic time.
    #[must_use]
    pub const fn from_elapsed(elapsed: Duration) -> Self {
        Self(elapsed)
    }

    /// Returns the elapsed monotonic time.
    #[must_use]
    pub const fn elapsed(self) -> Duration {
        self.0
    }

    #[cfg(feature = "alloc")]
    fn after(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration))
    }
}

/// Reliable RDP-UDP protocol version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolVersion {
    /// Original RDP-UDP with 500 ms minimum retransmission timeout.
    V1,
    /// RDP-UDP with 300 ms minimum retransmission timeout.
    V2,
    /// RDP-UDP2 data transfer after legacy RDP-UDP connection initialization.
    ///
    /// This version is reserved but unavailable until cookie-hash binding and
    /// the RDP-UDP2 data plane are implemented.
    V3,
}

impl ProtocolVersion {
    #[cfg(feature = "alloc")]
    const fn as_wire(self) -> u16 {
        match self {
            Self::V1 => 0x0001,
            Self::V2 => 0x0002,
            Self::V3 => 0x0101,
        }
    }

    #[cfg(feature = "alloc")]
    fn from_wire(value: u16) -> Option<Self> {
        match value {
            0x0001 => Some(Self::V1),
            0x0002 => Some(Self::V2),
            0x0101 => Some(Self::V3),
            _ => None,
        }
    }

    #[cfg(feature = "alloc")]
    const fn retransmit_timeout(self) -> Duration {
        match self {
            Self::V1 => Duration::from_millis(500),
            Self::V2 | Self::V3 => Duration::from_millis(300),
        }
    }
}

/// Configuration for a reliable RDP-UDP connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    /// Initial coded/source sequence number selected by the caller.
    pub initial_sequence_number: u32,
    /// Maximum protocol version advertised in the client SYN.
    ///
    /// Version 3 is rejected until security-cookie binding is implemented.
    pub max_version: ProtocolVersion,
    /// Maximum number of source packets this endpoint can buffer.
    pub receive_window_size: u16,
    /// Advertised source-payload MTU.
    pub mtu: u16,
    /// Maximum SYN and source-packet retransmissions before failure.
    pub max_retransmits: u8,
    /// Maximum buffered out-of-order payloads.
    pub max_reorder_buffer: usize,
    /// Maximum queued application messages awaiting consumption.
    pub max_delivered_messages: usize,
}

impl Config {
    /// Validates size and resource bounds before construction.
    pub fn validate(self) -> Result<Self, Error> {
        if !(MIN_MTU..=MAX_MTU).contains(&usize::from(self.mtu)) {
            return Err(Error::InvalidMtu);
        }
        if self.receive_window_size == 0 {
            return Err(Error::InvalidReceiveWindow);
        }
        if usize::from(self.receive_window_size) > MAX_ACK_VECTOR_SIZE {
            return Err(Error::InvalidReceiveWindow);
        }
        if self.max_version == ProtocolVersion::V3 {
            return Err(Error::UnsupportedVersion);
        }
        if !(3..=5).contains(&self.max_retransmits) {
            return Err(Error::InvalidRetransmitLimit);
        }
        if self.max_reorder_buffer < usize::from(self.receive_window_size).saturating_sub(1)
            || self.max_delivered_messages < usize::from(self.receive_window_size)
        {
            return Err(Error::InvalidBufferLimit);
        }
        Ok(self)
    }

    /// Returns the largest application payload permitted in a source packet.
    ///
    /// ACK metadata is not part of the negotiated payload MTU.
    #[cfg(feature = "alloc")]
    pub fn maximum_source_payload_size(self) -> Result<usize, Error> {
        let config = self.validate()?;
        Ok(usize::from(config.mtu))
    }
}

/// Errors produced while parsing or advancing reliable RDP-UDP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The configured MTU lies outside the protocol range.
    InvalidMtu,
    /// The configured receive window cannot be represented by an ACK vector.
    InvalidReceiveWindow,
    /// The configured retry limit is zero.
    InvalidRetransmitLimit,
    /// A configured queue capacity is zero.
    InvalidBufferLimit,
    /// An operation was not valid in the current transport state.
    InvalidState,
    /// The datagram was shorter than its declared structure.
    TruncatedDatagram,
    /// The datagram contained reserved or mutually incompatible flags.
    InvalidFlags,
    /// The peer requested lossy operation, which reliable mode does not support.
    UnsupportedTransportMode,
    /// The peer selected an unsupported protocol version.
    UnsupportedVersion,
    /// An RDP-UDP2 packet used an unsupported optional payload.
    UnsupportedRdpUdp2Payload,
    /// The peer advertised an MTU outside the protocol range.
    PeerInvalidMtu,
    /// The application payload exceeds the negotiated MTU.
    PayloadTooLarge,
    /// No more outstanding packets may be queued until the peer acknowledges data.
    SendWindowFull,
    /// The configured receive queues cannot hold the advertised window.
    ReceiveBufferFull,
    /// Handshake or data retransmission exhausted the configured retry budget.
    RetransmitLimitReached,
    /// No valid datagram was received from the peer within the keepalive timeout.
    PeerTimedOut,
}

/// Current reliable RDP-UDP lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    /// No SYN has been sent.
    Idle,
    /// Waiting for a server SYN+ACK.
    SynSent,
    /// Waiting for the client ACK that completes the server-side handshake.
    SynReceived {
        /// Version selected in the server SYN+ACK.
        version: ProtocolVersion,
    },
    /// The peer has acknowledged the client SYN and data can flow.
    Open {
        /// Version negotiated during the SYN exchange.
        version: ProtocolVersion,
    },
    /// The retry budget was exhausted.
    Failed,
}

/// Bytes the runtime must transmit in one UDP datagram.
#[cfg(feature = "alloc")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Datagram(pub Vec<u8>);

/// A reliable client-side RDP-UDP transport.
#[cfg(feature = "alloc")]
pub struct ReliableUdp {
    config: Config,
    state: State,
    next_coded_sequence: u32,
    next_source_sequence: u32,
    expected_source_sequence: u32,
    highest_received_source_sequence: u32,
    cumulative_acked: u32,
    peer_receive_window_size: u16,
    peer_ack_vector_window_size: u16,
    peer_initial_sequence: Option<u32>,
    pending: BTreeMap<u32, PendingPacket>,
    v3_pending: BTreeMap<u64, V3PendingPacket>,
    reorder_buffer: BTreeMap<u32, Vec<u8>>,
    v3_reorder_buffer: BTreeMap<u64, Vec<u8>>,
    delivered: VecDeque<Vec<u8>>,
    delivered_to_runtime: usize,
    next_v3_coded_sequence: u64,
    next_v3_channel_sequence: u64,
    expected_v3_channel_sequence: u64,
    last_v3_received_data_sequence: Option<u64>,
    syn_retries: u8,
    syn_deadline: Option<Timestamp>,
    syn_datagram: Option<Vec<u8>>,
    round_trip_time: Option<Duration>,
    last_peer_activity: Option<Timestamp>,
    last_transmitted: Option<Timestamp>,
    congestion_window: u16,
    slow_start_threshold: u16,
    congestion_avoidance_acks: u16,
    last_congestion_response: Option<Timestamp>,
    congestion_notification_pending: bool,
    cwr_pending: bool,
    cwr_source_sequence: Option<u32>,
    peer_ack_vector_reset_sequence: Option<u32>,
    processed_ack_vectors_since_reset: u8,
    pending_ack_of_acks_reset_sequence: Option<u32>,
}

#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
struct PendingPacket {
    data: Vec<u8>,
    retries: u8,
    timeout: Duration,
    deadline: Timestamp,
    last_sent_at: Timestamp,
}

#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
struct V3PendingPacket {
    data: Vec<u8>,
    channel_sequence: u64,
    retries: u8,
    deadline: Timestamp,
}

#[cfg(feature = "alloc")]
impl ReliableUdp {
    /// Creates an idle transport state machine.
    pub fn new(config: Config) -> Result<Self, Error> {
        let config = config.validate()?;
        Ok(Self {
            next_coded_sequence: config.initial_sequence_number.wrapping_add(1),
            next_source_sequence: config.initial_sequence_number.wrapping_add(1),
            expected_source_sequence: 0,
            highest_received_source_sequence: 0,
            cumulative_acked: config.initial_sequence_number,
            peer_receive_window_size: config.receive_window_size,
            peer_ack_vector_window_size: config.receive_window_size,
            config,
            state: State::Idle,
            peer_initial_sequence: None,
            pending: BTreeMap::new(),
            v3_pending: BTreeMap::new(),
            reorder_buffer: BTreeMap::new(),
            v3_reorder_buffer: BTreeMap::new(),
            delivered: VecDeque::new(),
            delivered_to_runtime: 0,
            next_v3_coded_sequence: u64::from(config.initial_sequence_number),
            next_v3_channel_sequence: u64::from(config.initial_sequence_number),
            expected_v3_channel_sequence: 0,
            last_v3_received_data_sequence: None,
            syn_retries: 0,
            syn_deadline: None,
            syn_datagram: None,
            round_trip_time: None,
            last_peer_activity: None,
            last_transmitted: None,
            congestion_window: 4,
            slow_start_threshold: u16::MAX,
            congestion_avoidance_acks: 0,
            last_congestion_response: None,
            congestion_notification_pending: false,
            cwr_pending: false,
            cwr_source_sequence: None,
            peer_ack_vector_reset_sequence: None,
            processed_ack_vectors_since_reset: 0,
            pending_ack_of_acks_reset_sequence: None,
        })
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// Returns the negotiated maximum application payload of a source packet.
    #[must_use]
    pub fn maximum_source_payload_size(&self) -> usize {
        usize::from(self.config.mtu)
    }

    /// Emits the initial client SYN.
    pub fn start(&mut self, now: Timestamp) -> Result<Datagram, Error> {
        if self.state != State::Idle {
            return Err(Error::InvalidState);
        }

        let datagram = self.encode_syn();
        self.state = State::SynSent;
        self.syn_deadline = Some(now.after(self.config.max_version.retransmit_timeout()));
        self.syn_datagram = Some(datagram.clone());
        self.last_transmitted = Some(now);
        Ok(Datagram(datagram))
    }

    /// Accepts an initial client SYN and emits the server SYN+ACK.
    ///
    /// The returned state machine remains in [`State::SynReceived`] until the
    /// client acknowledges the selected server sequence number.
    pub fn accept(config: Config, now: Timestamp, datagram: &[u8]) -> Result<(Self, Datagram), Error> {
        let header = Header::decode(datagram)?;
        validate_client_syn_header(header)?;

        let peer_syn = SynData::decode(datagram.get(COMMON_HEADER_SIZE..).ok_or(Error::TruncatedDatagram)?)?;
        if !(MIN_MTU..=MAX_MTU).contains(&usize::from(peer_syn.upstream_mtu))
            || !(MIN_MTU..=MAX_MTU).contains(&usize::from(peer_syn.downstream_mtu))
        {
            return Err(Error::PeerInvalidMtu);
        }

        let peer_version = parse_peer_version(datagram, header.flags)?;
        let version = if peer_version.as_wire() > config.max_version.as_wire() {
            config.max_version
        } else {
            peer_version
        };
        let mut config = config.validate()?;
        config.mtu = config.mtu.min(peer_syn.upstream_mtu).min(peer_syn.downstream_mtu);

        let mut transport = Self::new(config)?;
        transport.expected_source_sequence = peer_syn.initial_sequence_number.wrapping_add(1);
        transport.highest_received_source_sequence = peer_syn.initial_sequence_number;
        transport.peer_receive_window_size = header.receive_window_size;
        transport.peer_ack_vector_window_size = header.receive_window_size;
        transport.peer_initial_sequence = Some(peer_syn.initial_sequence_number);
        transport.expected_v3_channel_sequence = u64::from(peer_syn.initial_sequence_number);
        transport.last_v3_received_data_sequence = Some(u64::from(peer_syn.initial_sequence_number));
        transport.state = State::SynReceived { version };
        transport.syn_deadline = Some(now.after(version.retransmit_timeout()));
        let response = transport.encode_syn_ack(peer_syn.initial_sequence_number, version);
        transport.syn_datagram = Some(response.clone());
        transport.last_peer_activity = Some(now);
        transport.last_transmitted = Some(now);
        transport.congestion_window = transport.congestion_window.min(header.receive_window_size.max(1));

        Ok((transport, Datagram(response)))
    }

    /// Accepts one complete UDP datagram and returns immediate acknowledgements.
    pub fn handle_datagram(&mut self, now: Timestamp, datagram: &[u8]) -> Result<Vec<Datagram>, Error> {
        let datagrams = match self.state {
            State::SynSent => {
                let header = Header::decode(datagram)?;
                if header.flags & (FLAG_FEC | FLAG_SYN_LOSSY) != 0 {
                    return Err(Error::UnsupportedTransportMode);
                }
                self.handle_syn_ack(now, datagram, header)
            }
            State::SynReceived { version } => {
                let header = Header::decode(datagram)?;
                if header.flags & (FLAG_FEC | FLAG_SYN_LOSSY) != 0 {
                    return Err(Error::UnsupportedTransportMode);
                }
                if header.flags & FLAG_SYN != 0 {
                    validate_client_syn_header(header)?;
                    let peer_syn =
                        SynData::decode(datagram.get(COMMON_HEADER_SIZE..).ok_or(Error::TruncatedDatagram)?)?;
                    if self.peer_initial_sequence != Some(peer_syn.initial_sequence_number)
                        || parse_peer_version(datagram, header.flags)? != version
                    {
                        return Err(Error::InvalidFlags);
                    }
                    let response = self.syn_datagram.clone().ok_or(Error::InvalidState)?;
                    return Ok(vec![Datagram(response)]);
                }
                self.handle_server_handshake_ack(now, datagram, header, version)
            }
            State::Open {
                version: ProtocolVersion::V3,
            } => self.handle_v3_datagram(now, datagram),
            State::Open { .. } => {
                let header = Header::decode(datagram)?;
                if header.flags & FLAG_SYN_LOSSY != 0 {
                    return Err(Error::UnsupportedTransportMode);
                }
                if header.flags & FLAG_SYN != 0 {
                    if header.flags & FLAG_ACK == 0 {
                        return Ok(Vec::new());
                    }
                    let State::Open { version } = self.state else {
                        return Err(Error::InvalidState);
                    };
                    self.handle_duplicate_syn_ack(now, datagram, header, version)
                } else {
                    self.handle_open_datagram(now, datagram, header)
                }
            }
            State::Idle | State::Failed => Err(Error::InvalidState),
        }?;

        if !datagrams.is_empty() {
            self.last_transmitted = Some(now);
        }
        Ok(datagrams)
    }

    /// Queues one whole higher-layer message for reliable transmission.
    pub fn send(&mut self, now: Timestamp, data: Vec<u8>) -> Result<Datagram, Error> {
        let State::Open { version } = self.state else {
            return Err(Error::InvalidState);
        };
        if version == ProtocolVersion::V3 {
            return self.send_v3(now, data);
        }
        if data.len() > usize::from(self.config.mtu) {
            return Err(Error::PayloadTooLarge);
        }
        if !sequence_in_window(
            self.next_source_sequence,
            self.cumulative_acked,
            self.peer_receive_window_size,
        ) || self.pending.len() >= usize::from(self.congestion_window)
        {
            return Err(Error::SendWindowFull);
        }

        let source_sequence = self.next_source_sequence;
        let coded_sequence = self.next_coded_sequence;
        let cwr = self.take_cwr(source_sequence);
        let datagram = self.encode_source(coded_sequence, source_sequence, &data, cwr)?;
        self.next_coded_sequence = self.next_coded_sequence.wrapping_add(1);
        self.next_source_sequence = self.next_source_sequence.wrapping_add(1);

        let timeout = self.current_retransmit_timeout()?;
        let deadline = now.after(timeout);
        self.pending.insert(
            source_sequence,
            PendingPacket {
                data,
                retries: 0,
                timeout,
                deadline,
                last_sent_at: now,
            },
        );
        self.last_transmitted = Some(now);
        Ok(Datagram(datagram))
    }

    /// Returns the earliest deadline that the runtime should schedule.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Timestamp> {
        let pending = match self.state {
            State::Open {
                version: ProtocolVersion::V3,
            } => self.v3_pending.values().map(|packet| packet.deadline).min(),
            State::Idle | State::SynSent | State::SynReceived { .. } | State::Open { .. } | State::Failed => {
                self.pending.values().map(|packet| packet.deadline).min()
            }
        };
        let mut deadline = match (self.syn_deadline, pending) {
            (Some(syn), Some(data)) => Some(core::cmp::min(syn, data)),
            (Some(syn), None) => Some(syn),
            (None, Some(data)) => Some(data),
            (None, None) => None,
        };
        if matches!(self.state, State::Open { .. }) {
            if let Some(last_peer_activity) = self.last_peer_activity {
                deadline = Some(match deadline {
                    Some(current) => core::cmp::min(current, last_peer_activity.after(PEER_TIMEOUT)),
                    None => last_peer_activity.after(PEER_TIMEOUT),
                });
            }
            if let Some(last_transmitted) = self.last_transmitted {
                deadline = Some(match deadline {
                    Some(current) => core::cmp::min(current, last_transmitted.after(KEEPALIVE_INTERVAL)),
                    None => last_transmitted.after(KEEPALIVE_INTERVAL),
                });
            }
        }
        deadline
    }

    /// Emits retransmissions whose deadlines have elapsed.
    pub fn handle_timeout(&mut self, now: Timestamp) -> Result<Vec<Datagram>, Error> {
        if matches!(self.state, State::Open { .. })
            && self
                .last_peer_activity
                .is_some_and(|last_peer_activity| now >= last_peer_activity.after(PEER_TIMEOUT))
        {
            self.state = State::Failed;
            return Err(Error::PeerTimedOut);
        }

        if matches!(self.state, State::SynSent | State::SynReceived { .. })
            && let Some(deadline) = self.syn_deadline
            && now >= deadline
        {
            if self.syn_retries >= self.config.max_retransmits {
                self.state = State::Failed;
                self.syn_deadline = None;
                return Err(Error::RetransmitLimitReached);
            }
            self.syn_retries = self.syn_retries.saturating_add(1);
            let timeout = self.current_retransmit_timeout()?;
            self.syn_deadline = Some(now.after(timeout));
            let datagram = self.syn_datagram.clone().ok_or(Error::InvalidState)?;
            self.last_transmitted = Some(now);
            return Ok(vec![Datagram(datagram)]);
        }

        if self.state
            == (State::Open {
                version: ProtocolVersion::V3,
            })
        {
            return self.handle_v3_timeout(now);
        }

        let expired = self
            .pending
            .iter()
            .filter_map(|(&sequence, packet)| (now >= packet.deadline).then_some(sequence))
            .collect::<Vec<_>>();
        let mut datagrams = Vec::with_capacity(expired.len() + 1);
        for sequence in expired {
            datagrams.push(self.retransmit_source(now, sequence)?);
        }

        if datagrams.is_empty()
            && matches!(self.state, State::Open { .. })
            && self
                .last_transmitted
                .is_some_and(|last_transmitted| now >= last_transmitted.after(KEEPALIVE_INTERVAL))
        {
            datagrams.push(Datagram(self.encode_ack()?));
        }
        if !datagrams.is_empty() {
            self.last_transmitted = Some(now);
        }

        Ok(datagrams)
    }

    /// Removes and returns the next in-order higher-layer message.
    pub fn take_received(&mut self) -> Option<Vec<u8>> {
        self.delivered.pop_front()
    }

    /// Returns whether an in-order higher-layer message is ready for delivery.
    #[must_use]
    pub fn has_received(&self) -> bool {
        !self.delivered.is_empty()
    }

    /// Removes one message for deferred runtime delivery.
    ///
    /// The message continues to consume receive-window capacity until
    /// [`Self::release_runtime_delivery`] records that the higher layer read it.
    pub fn take_received_for_runtime(&mut self) -> Option<Vec<u8>> {
        let message = self.delivered.pop_front()?;
        self.delivered_to_runtime = self.delivered_to_runtime.saturating_add(1);
        Some(message)
    }

    /// Releases receive-window capacity after the runtime delivers messages to its user.
    pub fn release_runtime_delivery(&mut self, count: usize) -> Result<(), Error> {
        if count > self.delivered_to_runtime {
            return Err(Error::InvalidState);
        }
        self.delivered_to_runtime -= count;
        Ok(())
    }

    /// Emits an acknowledgement after the runtime released receive-window capacity.
    pub fn acknowledge_receive_window(&mut self, now: Timestamp) -> Result<Datagram, Error> {
        if !matches!(
            self.state,
            State::Open {
                version: ProtocolVersion::V1 | ProtocolVersion::V2
            }
        ) {
            return Err(Error::InvalidState);
        }

        let acknowledgement = Datagram(self.encode_ack()?);
        self.last_transmitted = Some(now);
        Ok(acknowledgement)
    }

    fn handle_syn_ack(&mut self, now: Timestamp, datagram: &[u8], header: Header) -> Result<Vec<Datagram>, Error> {
        const SYN_ACK_FLAGS: u16 = FLAG_SYN | FLAG_ACK | FLAG_CORRELATION_ID | FLAG_SYN_EX;

        if header.flags & !SYN_ACK_FLAGS != 0
            || header.flags & (FLAG_SYN | FLAG_ACK) != (FLAG_SYN | FLAG_ACK)
            || header.source_ack != self.config.initial_sequence_number
        {
            return Err(Error::InvalidFlags);
        }
        let syn = SynData::decode(datagram.get(COMMON_HEADER_SIZE..).ok_or(Error::TruncatedDatagram)?)?;
        let version = parse_peer_version(datagram, header.flags)?;
        if version.as_wire() > self.config.max_version.as_wire() {
            return Err(Error::UnsupportedVersion);
        }
        if usize::from(syn.upstream_mtu) < MIN_MTU
            || usize::from(syn.upstream_mtu) > MAX_MTU
            || usize::from(syn.downstream_mtu) < MIN_MTU
            || usize::from(syn.downstream_mtu) > MAX_MTU
        {
            return Err(Error::PeerInvalidMtu);
        }

        self.config.mtu = self.config.mtu.min(syn.upstream_mtu).min(syn.downstream_mtu);
        self.expected_source_sequence = syn.initial_sequence_number.wrapping_add(1);
        self.highest_received_source_sequence = syn.initial_sequence_number;
        self.peer_receive_window_size = header.receive_window_size;
        self.peer_ack_vector_window_size = header.receive_window_size;
        self.peer_initial_sequence = Some(syn.initial_sequence_number);
        self.expected_v3_channel_sequence = u64::from(syn.initial_sequence_number);
        self.last_v3_received_data_sequence = Some(u64::from(syn.initial_sequence_number));
        self.state = State::Open { version };
        self.syn_deadline = None;
        self.syn_datagram = None;
        self.last_peer_activity = Some(now);
        self.congestion_window = self.congestion_window.min(header.receive_window_size.max(1));
        let ack = self.encode_ack()?;
        Ok(vec![Datagram(ack)])
    }

    fn handle_server_handshake_ack(
        &mut self,
        now: Timestamp,
        datagram: &[u8],
        header: Header,
        version: ProtocolVersion,
    ) -> Result<Vec<Datagram>, Error> {
        if header.flags & FLAG_ACK == 0
            || header.flags & (FLAG_SYN | FLAG_SYN_EX) != 0
            || header.source_ack != self.config.initial_sequence_number
        {
            return Err(Error::InvalidFlags);
        }

        let datagrams = self.handle_open_datagram(now, datagram, header)?;
        self.state = State::Open { version };
        self.syn_deadline = None;
        self.syn_datagram = None;
        Ok(datagrams)
    }

    fn handle_duplicate_syn_ack(
        &mut self,
        now: Timestamp,
        datagram: &[u8],
        header: Header,
        version: ProtocolVersion,
    ) -> Result<Vec<Datagram>, Error> {
        const SYN_ACK_FLAGS: u16 = FLAG_SYN | FLAG_ACK | FLAG_CORRELATION_ID | FLAG_SYN_EX;

        if header.flags & !SYN_ACK_FLAGS != 0
            || header.flags & (FLAG_SYN | FLAG_ACK) != (FLAG_SYN | FLAG_ACK)
            || header.source_ack != self.config.initial_sequence_number
        {
            return Err(Error::InvalidFlags);
        }

        let syn = SynData::decode(datagram.get(COMMON_HEADER_SIZE..).ok_or(Error::TruncatedDatagram)?)?;
        if !(MIN_MTU..=MAX_MTU).contains(&usize::from(syn.upstream_mtu))
            || !(MIN_MTU..=MAX_MTU).contains(&usize::from(syn.downstream_mtu))
            || self.peer_initial_sequence != Some(syn.initial_sequence_number)
            || parse_peer_version(datagram, header.flags)? != version
        {
            return Err(Error::InvalidFlags);
        }

        self.last_peer_activity = Some(now);
        Ok(vec![Datagram(self.encode_ack()?)])
    }

    fn handle_open_datagram(
        &mut self,
        now: Timestamp,
        datagram: &[u8],
        header: Header,
    ) -> Result<Vec<Datagram>, Error> {
        const OPEN_FLAGS: u16 =
            FLAG_ACK | FLAG_DATA | FLAG_FEC | FLAG_CN | FLAG_CWR | FLAG_ACK_OF_ACKS | FLAG_ACK_DELAYED;

        if header.flags & !OPEN_FLAGS != 0 || header.flags & (FLAG_SYN | FLAG_SYN_EX | FLAG_SYN_LOSSY) != 0 {
            return Err(Error::InvalidFlags);
        }
        if header.flags & FLAG_ACK == 0 {
            return Err(Error::InvalidFlags);
        }
        if header.flags & FLAG_FEC != 0 && header.flags & FLAG_DATA == 0 {
            return Err(Error::InvalidFlags);
        }
        if datagram.len() > MAX_DATAGRAM_SIZE {
            return Err(Error::PayloadTooLarge);
        }

        let mut offset = COMMON_HEADER_SIZE;
        let (ack_vector, ack_vector_size) = decode_ack_vector(
            datagram.get(offset..).ok_or(Error::TruncatedDatagram)?,
            usize::from(self.peer_ack_vector_window_size),
        )?;
        offset += ack_vector_size;
        let ack_of_acks_reset_sequence = if header.flags & FLAG_ACK_OF_ACKS != 0 {
            let bytes = datagram.get(offset..offset + 4).ok_or(Error::TruncatedDatagram)?;
            offset += 4;
            Some(u32::from_be_bytes(
                bytes.try_into().map_err(|_| Error::TruncatedDatagram)?,
            ))
        } else {
            None
        };
        if header.flags & FLAG_DATA == 0 {
            if offset != datagram.len() {
                return Err(Error::InvalidFlags);
            }
            let outgoing = self.process_open_ack(now, header, &ack_vector, ack_of_acks_reset_sequence)?;
            self.last_peer_activity = Some(now);
            return Ok(outgoing);
        }
        if header.flags & FLAG_FEC != 0 {
            let fec = datagram.get(offset..).ok_or(Error::TruncatedDatagram)?;
            validate_fec_payload(fec)?;
            let outgoing = self.process_open_ack(now, header, &ack_vector, ack_of_acks_reset_sequence)?;
            self.last_peer_activity = Some(now);
            return Ok(outgoing);
        }

        let source = SourcePayload::decode(datagram.get(offset..).ok_or(Error::TruncatedDatagram)?)?;
        if source.data.len() > usize::from(self.config.mtu) {
            return Ok(Vec::new());
        }
        let mut outgoing = self.process_open_ack(now, header, &ack_vector, ack_of_acks_reset_sequence)?;
        if self.receive_source(source)? {
            outgoing.push(Datagram(self.encode_ack()?));
        }
        self.last_peer_activity = Some(now);
        Ok(outgoing)
    }

    fn process_open_ack(
        &mut self,
        now: Timestamp,
        header: Header,
        ack_vector: &[AckVectorState],
        ack_of_acks_reset_sequence: Option<u32>,
    ) -> Result<Vec<Datagram>, Error> {
        if sequence_after(header.source_ack, self.next_source_sequence.wrapping_sub(1)) {
            return Ok(Vec::new());
        }
        self.peer_receive_window_size = header.receive_window_size;
        if let Some(reset_sequence) = ack_of_acks_reset_sequence {
            self.set_peer_ack_vector_reset_sequence(reset_sequence)?;
        }
        let outgoing = self.process_peer_ack(now, header, ack_vector)?;
        if header.flags & FLAG_CWR != 0 {
            self.congestion_notification_pending = false;
        }
        Ok(outgoing)
    }

    fn handle_v3_datagram(&mut self, now: Timestamp, datagram: &[u8]) -> Result<Vec<Datagram>, Error> {
        let Some(layout) = decode_v3_wire_packet(datagram)? else {
            return Ok(Vec::new());
        };
        let header = V3Header::decode(&layout)?;
        if header.flags & V3_FLAG_ACKVEC != 0 {
            return Err(Error::UnsupportedRdpUdp2Payload);
        }
        if header.flags & !V3_SUPPORTED_FLAGS != 0 {
            return Err(Error::UnsupportedRdpUdp2Payload);
        }
        if header.flags == 0 {
            return Err(Error::InvalidFlags);
        }

        let mut offset = V3_HEADER_SIZE;
        if header.flags & V3_FLAG_ACK != 0 {
            let ack = V3AckPayload::decode(layout.get(offset..).ok_or(Error::TruncatedDatagram)?)?;
            offset += V3_ACK_SIZE;
            let reference = self.next_v3_coded_sequence.saturating_sub(1);
            self.acknowledge_v3(reconstruct_sequence_16(reference, ack.sequence));
        }

        if header.flags & V3_FLAG_DATA == 0 {
            if offset != layout.len() {
                return Err(Error::InvalidFlags);
            }
            return Ok(Vec::new());
        }

        let data = V3DataPayload::decode(layout.get(offset..).ok_or(Error::TruncatedDatagram)?)?;
        if data.data.len() + V3_WIRE_OVERHEAD > usize::from(self.config.mtu) {
            return Err(Error::PayloadTooLarge);
        }
        let data_reference = self
            .last_v3_received_data_sequence
            .unwrap_or(self.expected_v3_channel_sequence);
        let data_sequence = reconstruct_sequence_16(data_reference, data.data_sequence);
        if self
            .last_v3_received_data_sequence
            .is_none_or(|previous| data_sequence > previous)
        {
            self.last_v3_received_data_sequence = Some(data_sequence);
        }
        let channel_sequence = reconstruct_sequence_16(self.expected_v3_channel_sequence, data.channel_sequence);
        self.receive_v3_source(channel_sequence, data.data)?;
        Ok(vec![Datagram(self.encode_v3_ack(now, data_sequence))])
    }

    fn send_v3(&mut self, now: Timestamp, data: Vec<u8>) -> Result<Datagram, Error> {
        if data.len() + V3_WIRE_OVERHEAD > usize::from(self.config.mtu) {
            return Err(Error::PayloadTooLarge);
        }
        if self.v3_pending.len() >= usize::from(self.config.receive_window_size) {
            return Err(Error::SendWindowFull);
        }

        let coded_sequence = self.next_v3_coded_sequence;
        let channel_sequence = self.next_v3_channel_sequence;
        let datagram = self.encode_v3_data(coded_sequence, channel_sequence, &data);
        self.next_v3_coded_sequence = self.next_v3_coded_sequence.saturating_add(1);
        self.next_v3_channel_sequence = self.next_v3_channel_sequence.saturating_add(1);
        self.v3_pending.insert(
            coded_sequence,
            V3PendingPacket {
                data,
                channel_sequence,
                retries: 0,
                deadline: now.after(self.current_retransmit_timeout()?),
            },
        );
        Ok(Datagram(datagram))
    }

    fn handle_v3_timeout(&mut self, now: Timestamp) -> Result<Vec<Datagram>, Error> {
        let timeout = self.current_retransmit_timeout()?;
        let expired = self
            .v3_pending
            .iter()
            .filter_map(|(&sequence, packet)| (now >= packet.deadline).then_some(sequence))
            .collect::<Vec<_>>();
        let mut datagrams = Vec::with_capacity(expired.len());
        for sequence in expired {
            let packet = self.v3_pending.remove(&sequence).ok_or(Error::InvalidState)?;
            if packet.retries >= self.config.max_retransmits {
                self.state = State::Failed;
                return Err(Error::RetransmitLimitReached);
            }
            let coded_sequence = self.next_v3_coded_sequence;
            self.next_v3_coded_sequence = self.next_v3_coded_sequence.saturating_add(1);
            let datagram = self.encode_v3_data(coded_sequence, packet.channel_sequence, &packet.data);
            self.v3_pending.insert(
                coded_sequence,
                V3PendingPacket {
                    deadline: now.after(timeout),
                    retries: packet.retries.saturating_add(1),
                    ..packet
                },
            );
            datagrams.push(Datagram(datagram));
        }
        Ok(datagrams)
    }

    fn receive_source(&mut self, source: SourcePayload<'_>) -> Result<bool, Error> {
        if sequence_before_or_equal(source.source_sequence, self.expected_source_sequence.wrapping_sub(1)) {
            return Ok(true);
        }

        if !sequence_in_receive_window(
            source.source_sequence,
            self.expected_source_sequence,
            self.available_receive_window(),
        ) {
            return Ok(false);
        }

        if source.source_sequence == self.expected_source_sequence {
            self.push_delivered(source.data.to_vec())?;
            self.expected_source_sequence = self.expected_source_sequence.wrapping_add(1);
            while let Some(data) = self.reorder_buffer.remove(&self.expected_source_sequence) {
                self.push_delivered(data)?;
                self.expected_source_sequence = self.expected_source_sequence.wrapping_add(1);
            }
        } else if !self.reorder_buffer.contains_key(&source.source_sequence) {
            if self.reorder_buffer.len() >= self.config.max_reorder_buffer {
                return Err(Error::ReceiveBufferFull);
            }
            self.reorder_buffer
                .entry(source.source_sequence)
                .or_insert_with(|| source.data.to_vec());
        }
        if sequence_after(source.source_sequence, self.highest_received_source_sequence) {
            self.highest_received_source_sequence = source.source_sequence;
        }
        if self
            .reorder_buffer
            .keys()
            .filter(|&&sequence| sequence_after(sequence, self.expected_source_sequence))
            .take(3)
            .count()
            >= 3
        {
            self.congestion_notification_pending = true;
        }
        Ok(true)
    }

    fn receive_v3_source(&mut self, channel_sequence: u64, data: &[u8]) -> Result<(), Error> {
        if channel_sequence == self.expected_v3_channel_sequence {
            self.push_delivered(data.to_vec())?;
            self.expected_v3_channel_sequence = self.expected_v3_channel_sequence.saturating_add(1);
            while let Some(data) = self.v3_reorder_buffer.remove(&self.expected_v3_channel_sequence) {
                self.push_delivered(data)?;
                self.expected_v3_channel_sequence = self.expected_v3_channel_sequence.saturating_add(1);
            }
        } else if channel_sequence > self.expected_v3_channel_sequence {
            let distance = channel_sequence - self.expected_v3_channel_sequence;
            if distance > u64::try_from(self.config.max_reorder_buffer).map_err(|_| Error::ReceiveBufferFull)?
                || (self.v3_reorder_buffer.len() >= self.config.max_reorder_buffer
                    && !self.v3_reorder_buffer.contains_key(&channel_sequence))
            {
                return Err(Error::ReceiveBufferFull);
            }
            self.v3_reorder_buffer
                .entry(channel_sequence)
                .or_insert_with(|| data.to_vec());
        }
        Ok(())
    }

    fn push_delivered(&mut self, data: Vec<u8>) -> Result<(), Error> {
        if self.delivered.len() >= self.config.max_delivered_messages {
            return Err(Error::ReceiveBufferFull);
        }
        self.delivered.push_back(data);
        Ok(())
    }

    fn acknowledge_v3(&mut self, data_ack: u64) {
        self.v3_pending.retain(|&sequence, _| sequence > data_ack);
    }

    fn process_peer_ack(
        &mut self,
        now: Timestamp,
        header: Header,
        ack_vector: &[AckVectorState],
    ) -> Result<Vec<Datagram>, Error> {
        let last_sent_sequence = self.next_source_sequence.wrapping_sub(1);
        if sequence_after(header.source_ack, last_sent_sequence) {
            return Ok(Vec::new());
        }

        let vector_start = if ack_vector.is_empty() {
            header.source_ack.wrapping_add(1)
        } else {
            header
                .source_ack
                .wrapping_sub(u32::try_from(ack_vector.len() - 1).map_err(|_| Error::InvalidFlags)?)
        };
        let implicit_ack = if ack_vector.is_empty() {
            header.source_ack
        } else {
            vector_start.wrapping_sub(1)
        };
        let delayed = header.flags & FLAG_ACK_DELAYED != 0;
        let mut acknowledged = self.acknowledge_through(now, implicit_ack, delayed, last_sent_sequence);

        for (offset, state) in ack_vector.iter().enumerate() {
            if *state != AckVectorState::Received {
                continue;
            }
            let sequence = vector_start.wrapping_add(u32::try_from(offset).map_err(|_| Error::InvalidFlags)?);
            if !sequence_after(sequence, last_sent_sequence) {
                acknowledged += u16::from(self.acknowledge_source(now, sequence, delayed));
            }
        }
        self.advance_cumulative_ack(last_sent_sequence);
        self.increase_congestion_window(acknowledged);
        self.processed_ack_vectors_since_reset = self.processed_ack_vectors_since_reset.saturating_add(1);
        if self.processed_ack_vectors_since_reset >= 20 {
            self.pending_ack_of_acks_reset_sequence = Some(self.cumulative_acked);
            self.processed_ack_vectors_since_reset = 0;
        }

        if header.flags & FLAG_CN != 0
            && !self
                .cwr_source_sequence
                .is_some_and(|cwr_sequence| sequence_after(cwr_sequence, header.source_ack))
        {
            self.react_to_congestion(now);
        }

        let mut received_after = 0usize;
        let mut fast_retransmits = Vec::new();
        for (offset, state) in ack_vector.iter().enumerate().rev() {
            let sequence = vector_start.wrapping_add(u32::try_from(offset).map_err(|_| Error::InvalidFlags)?);
            match state {
                AckVectorState::Received => received_after = received_after.saturating_add(1),
                AckVectorState::NotYetReceived
                    if received_after >= 3 && self.pending.get(&sequence).is_some_and(|packet| packet.retries == 0) =>
                {
                    fast_retransmits.push(sequence);
                }
                AckVectorState::NotYetReceived => {}
            }
        }
        if !fast_retransmits.is_empty() {
            self.react_to_congestion(now);
        }

        let mut retransmissions = Vec::with_capacity(fast_retransmits.len());
        for sequence in fast_retransmits {
            retransmissions.push(self.retransmit_source(now, sequence)?);
        }
        Ok(retransmissions)
    }

    fn set_peer_ack_vector_reset_sequence(&mut self, reset_sequence: u32) -> Result<(), Error> {
        if sequence_after(reset_sequence, self.highest_received_source_sequence) {
            return Err(Error::InvalidFlags);
        }
        if self
            .peer_ack_vector_reset_sequence
            .is_none_or(|current_reset_sequence| sequence_after(reset_sequence, current_reset_sequence))
        {
            self.peer_ack_vector_reset_sequence = Some(reset_sequence);
        }
        Ok(())
    }

    fn acknowledge_through(&mut self, now: Timestamp, sequence: u32, delayed: bool, last_sent_sequence: u32) -> u16 {
        let mut acknowledged = 0;
        while sequence_after(last_sent_sequence, self.cumulative_acked)
            && sequence_before_or_equal(self.cumulative_acked.wrapping_add(1), sequence)
        {
            let next = self.cumulative_acked.wrapping_add(1);
            acknowledged += u16::from(self.acknowledge_source(now, next, delayed));
            self.cumulative_acked = next;
        }
        acknowledged
    }

    fn acknowledge_source(&mut self, now: Timestamp, sequence: u32, delayed: bool) -> bool {
        let Some(packet) = self.pending.remove(&sequence) else {
            return false;
        };
        if packet.retries == 0 && !delayed {
            self.round_trip_time = Some(now.elapsed().saturating_sub(packet.last_sent_at.elapsed()));
        }
        true
    }

    fn advance_cumulative_ack(&mut self, last_sent_sequence: u32) {
        while sequence_after(last_sent_sequence, self.cumulative_acked)
            && !self.pending.contains_key(&self.cumulative_acked.wrapping_add(1))
        {
            self.cumulative_acked = self.cumulative_acked.wrapping_add(1);
        }
    }

    fn increase_congestion_window(&mut self, acknowledged: u16) {
        if acknowledged == 0 {
            return;
        }
        if self.congestion_window < self.slow_start_threshold {
            self.congestion_window = self.congestion_window.saturating_add(acknowledged);
            return;
        }

        self.congestion_avoidance_acks = self.congestion_avoidance_acks.saturating_add(acknowledged);
        while self.congestion_avoidance_acks >= self.congestion_window {
            self.congestion_avoidance_acks -= self.congestion_window;
            self.congestion_window = self.congestion_window.saturating_add(1);
        }
    }

    fn react_to_congestion(&mut self, now: Timestamp) {
        let reaction_interval = self
            .round_trip_time
            .unwrap_or_else(|| self.current_retransmit_timeout().unwrap_or_default());
        if self
            .last_congestion_response
            .is_none_or(|last_response| now >= last_response.after(reaction_interval))
        {
            self.slow_start_threshold = core::cmp::max(1, self.congestion_window / 2);
            self.congestion_window = self.slow_start_threshold;
            self.congestion_avoidance_acks = 0;
            self.last_congestion_response = Some(now);
        }
        self.cwr_pending = true;
    }

    fn retransmit_source(&mut self, now: Timestamp, source_sequence: u32) -> Result<Datagram, Error> {
        let (data, retries, previous_timeout) = {
            let packet = self.pending.get(&source_sequence).ok_or(Error::InvalidState)?;
            (packet.data.clone(), packet.retries, packet.timeout)
        };
        if retries >= self.config.max_retransmits {
            self.state = State::Failed;
            return Err(Error::RetransmitLimitReached);
        }

        self.react_to_congestion(now);
        let coded_sequence = self.next_coded_sequence;
        self.next_coded_sequence = self.next_coded_sequence.wrapping_add(1);
        let datagram = self.encode_source(coded_sequence, source_sequence, &data, true)?;
        let timeout = core::cmp::max(previous_timeout, self.current_retransmit_timeout()?);
        let packet = self.pending.get_mut(&source_sequence).ok_or(Error::InvalidState)?;
        packet.retries = packet.retries.saturating_add(1);
        packet.timeout = timeout;
        packet.deadline = now.after(timeout);
        packet.last_sent_at = now;
        self.cwr_source_sequence = Some(source_sequence);
        self.cwr_pending = false;
        Ok(Datagram(datagram))
    }

    fn available_receive_window(&self) -> u16 {
        let used = self
            .reorder_buffer
            .len()
            .saturating_add(self.delivered.len())
            .saturating_add(self.delivered_to_runtime);
        let available = usize::from(self.config.receive_window_size).saturating_sub(used);
        u16::try_from(available).unwrap_or_default()
    }

    fn take_cwr(&mut self, source_sequence: u32) -> bool {
        if !self.cwr_pending {
            return false;
        }
        self.cwr_pending = false;
        self.cwr_source_sequence = Some(source_sequence);
        true
    }

    fn current_retransmit_timeout(&self) -> Result<Duration, Error> {
        let minimum = match self.state {
            State::Open { version } => version.retransmit_timeout(),
            State::SynSent => self.config.max_version.retransmit_timeout(),
            State::SynReceived { version } => version.retransmit_timeout(),
            State::Idle | State::Failed => return Err(Error::InvalidState),
        };
        Ok(self
            .round_trip_time
            .map_or(minimum, |rtt| core::cmp::max(minimum, rtt.saturating_mul(2))))
    }

    fn encode_syn(&self) -> Vec<u8> {
        let mut datagram = Vec::with_capacity(COMMON_HEADER_SIZE + SYN_DATA_SIZE + 4);
        Header {
            source_ack: u32::MAX,
            receive_window_size: self.config.receive_window_size,
            flags: FLAG_SYN | FLAG_SYN_EX,
        }
        .encode(&mut datagram);
        SynData {
            initial_sequence_number: self.config.initial_sequence_number,
            upstream_mtu: self.config.mtu,
            downstream_mtu: self.config.mtu,
        }
        .encode(&mut datagram);
        datagram.extend_from_slice(&SYN_EX_VERSION_INFO_VALID.to_be_bytes());
        datagram.extend_from_slice(&self.config.max_version.as_wire().to_be_bytes());
        datagram.resize(usize::from(self.config.mtu), 0);
        datagram
    }

    fn encode_syn_ack(&self, peer_initial_sequence: u32, version: ProtocolVersion) -> Vec<u8> {
        let mut datagram = Vec::with_capacity(COMMON_HEADER_SIZE + SYN_DATA_SIZE + 4);
        let flags = if version == ProtocolVersion::V1 {
            FLAG_SYN | FLAG_ACK
        } else {
            FLAG_SYN | FLAG_ACK | FLAG_SYN_EX
        };
        Header {
            source_ack: peer_initial_sequence,
            receive_window_size: self.config.receive_window_size,
            flags,
        }
        .encode(&mut datagram);
        SynData {
            initial_sequence_number: self.config.initial_sequence_number,
            upstream_mtu: self.config.mtu,
            downstream_mtu: self.config.mtu,
        }
        .encode(&mut datagram);
        if version != ProtocolVersion::V1 {
            datagram.extend_from_slice(&SYN_EX_VERSION_INFO_VALID.to_be_bytes());
            datagram.extend_from_slice(&version.as_wire().to_be_bytes());
        }
        datagram.resize(usize::from(self.config.mtu), 0);
        datagram
    }

    fn encode_ack(&mut self) -> Result<Vec<u8>, Error> {
        let ack_vector = self.encode_ack_vector()?;
        let ack_of_acks_reset_sequence = self.pending_ack_of_acks_reset_sequence.take();
        let flags = FLAG_ACK
            | if self.congestion_notification_pending {
                FLAG_CN
            } else {
                0
            }
            | if ack_of_acks_reset_sequence.is_some() {
                FLAG_ACK_OF_ACKS
            } else {
                0
            };
        let mut datagram =
            Vec::with_capacity(COMMON_HEADER_SIZE + ack_vector.len() + ack_of_acks_reset_sequence.map_or(0, |_| 4));
        Header {
            source_ack: self.highest_received_source_sequence,
            receive_window_size: self.available_receive_window(),
            flags,
        }
        .encode(&mut datagram);
        datagram.extend_from_slice(&ack_vector);
        if let Some(reset_sequence) = ack_of_acks_reset_sequence {
            datagram.extend_from_slice(&reset_sequence.to_be_bytes());
        }
        Ok(datagram)
    }

    fn encode_source(
        &mut self,
        coded_sequence: u32,
        source_sequence: u32,
        data: &[u8],
        cwr: bool,
    ) -> Result<Vec<u8>, Error> {
        let ack_vector = self.encode_ack_vector()?;
        let ack_of_acks_reset_sequence = self.pending_ack_of_acks_reset_sequence.take();
        let flags = FLAG_ACK
            | FLAG_DATA
            | if self.congestion_notification_pending {
                FLAG_CN
            } else {
                0
            }
            | if ack_of_acks_reset_sequence.is_some() {
                FLAG_ACK_OF_ACKS
            } else {
                0
            }
            | if cwr { FLAG_CWR } else { 0 };
        let mut datagram = Vec::with_capacity(
            COMMON_HEADER_SIZE
                + ack_vector.len()
                + ack_of_acks_reset_sequence.map_or(0, |_| 4)
                + SOURCE_HEADER_SIZE
                + data.len(),
        );
        Header {
            source_ack: self.highest_received_source_sequence,
            receive_window_size: self.available_receive_window(),
            flags,
        }
        .encode(&mut datagram);
        datagram.extend_from_slice(&ack_vector);
        if let Some(reset_sequence) = ack_of_acks_reset_sequence {
            datagram.extend_from_slice(&reset_sequence.to_be_bytes());
        }
        datagram.extend_from_slice(&coded_sequence.to_be_bytes());
        datagram.extend_from_slice(&source_sequence.to_be_bytes());
        datagram.extend_from_slice(data);
        if datagram.len() > MAX_DATAGRAM_SIZE {
            return Err(Error::PayloadTooLarge);
        }
        Ok(datagram)
    }

    fn encode_ack_vector(&self) -> Result<Vec<u8>, Error> {
        let mut encoded = Vec::new();
        let first_sequence =
            self.peer_ack_vector_reset_sequence
                .map_or(self.expected_source_sequence, |reset_sequence| {
                    let reset_next_sequence = reset_sequence.wrapping_add(1);
                    if sequence_after(reset_next_sequence, self.expected_source_sequence) {
                        reset_next_sequence
                    } else {
                        self.expected_source_sequence
                    }
                });
        if sequence_after_or_equal(self.highest_received_source_sequence, first_sequence) {
            let length = usize::try_from(
                self.highest_received_source_sequence
                    .wrapping_sub(first_sequence)
                    .wrapping_add(1),
            )
            .map_err(|_| Error::InvalidFlags)?;
            if length > usize::from(self.config.receive_window_size) {
                return Err(Error::InvalidFlags);
            }

            let mut sequence = first_sequence;
            let mut run_state = if self.reorder_buffer.contains_key(&sequence) {
                AckVectorState::Received
            } else {
                AckVectorState::NotYetReceived
            };
            let mut run_length = 0u8;
            for _ in 0..length {
                let state = if self.reorder_buffer.contains_key(&sequence) {
                    AckVectorState::Received
                } else {
                    AckVectorState::NotYetReceived
                };
                if state != run_state || run_length == 63 {
                    encoded.push(run_state.encode(run_length));
                    run_state = state;
                    run_length = 0;
                }
                run_length = run_length.saturating_add(1);
                sequence = sequence.wrapping_add(1);
            }
            encoded.push(run_state.encode(run_length));
        }
        if encoded.len() > MAX_ACK_VECTOR_SIZE {
            return Err(Error::InvalidFlags);
        }

        let mut vector = Vec::with_capacity(ACK_VECTOR_HEADER_SIZE + encoded.len() + 3);
        vector.extend_from_slice(
            &u16::try_from(encoded.len())
                .map_err(|_| Error::InvalidFlags)?
                .to_be_bytes(),
        );
        vector.extend_from_slice(&encoded);
        vector.resize(
            ACK_VECTOR_HEADER_SIZE + encoded.len() + ack_vector_padding(encoded.len()),
            0,
        );
        Ok(vector)
    }

    fn encode_v3_data(&self, data_sequence: u64, channel_sequence: u64, data: &[u8]) -> Vec<u8> {
        let mut layout = Vec::with_capacity(V3_HEADER_SIZE + V3_DATA_HEADER_SIZE + V3_CHANNEL_HEADER_SIZE + data.len());
        V3Header {
            flags: V3_FLAG_DATA,
            log_window_size: v3_log_window_size(self.config.receive_window_size),
        }
        .encode(&mut layout);
        layout.extend_from_slice(&low_sequence_bytes(data_sequence));
        layout.extend_from_slice(&low_sequence_bytes(channel_sequence));
        layout.extend_from_slice(data);
        encode_v3_wire_packet(layout)
    }

    fn encode_v3_ack(&self, now: Timestamp, data_sequence: u64) -> Vec<u8> {
        let mut layout = Vec::with_capacity(V3_HEADER_SIZE + V3_ACK_SIZE);
        V3Header {
            flags: V3_FLAG_ACK,
            log_window_size: v3_log_window_size(self.config.receive_window_size),
        }
        .encode(&mut layout);
        V3AckPayload {
            sequence: low_sequence(data_sequence),
            received_timestamp: timestamp_4us(now),
        }
        .encode(&mut layout);
        encode_v3_wire_packet(layout)
    }
}

#[cfg(feature = "alloc")]
#[derive(Clone, Copy)]
struct Header {
    source_ack: u32,
    receive_window_size: u16,
    flags: u16,
}

#[cfg(feature = "alloc")]
impl Header {
    fn decode(datagram: &[u8]) -> Result<Self, Error> {
        let bytes: [u8; COMMON_HEADER_SIZE] = datagram
            .get(..COMMON_HEADER_SIZE)
            .ok_or(Error::TruncatedDatagram)?
            .try_into()
            .map_err(|_| Error::TruncatedDatagram)?;
        Ok(Self {
            source_ack: u32::from_be_bytes(bytes[0..4].try_into().map_err(|_| Error::TruncatedDatagram)?),
            receive_window_size: u16::from_be_bytes(bytes[4..6].try_into().map_err(|_| Error::TruncatedDatagram)?),
            flags: u16::from_be_bytes(bytes[6..8].try_into().map_err(|_| Error::TruncatedDatagram)?),
        })
    }

    fn encode(self, datagram: &mut Vec<u8>) {
        datagram.extend_from_slice(&self.source_ack.to_be_bytes());
        datagram.extend_from_slice(&self.receive_window_size.to_be_bytes());
        datagram.extend_from_slice(&self.flags.to_be_bytes());
    }
}

#[cfg(feature = "alloc")]
#[derive(Clone, Copy)]
struct SynData {
    initial_sequence_number: u32,
    upstream_mtu: u16,
    downstream_mtu: u16,
}

#[cfg(feature = "alloc")]
impl SynData {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let bytes: [u8; SYN_DATA_SIZE] = bytes
            .get(..SYN_DATA_SIZE)
            .ok_or(Error::TruncatedDatagram)?
            .try_into()
            .map_err(|_| Error::TruncatedDatagram)?;
        Ok(Self {
            initial_sequence_number: u32::from_be_bytes(bytes[0..4].try_into().map_err(|_| Error::TruncatedDatagram)?),
            upstream_mtu: u16::from_be_bytes(bytes[4..6].try_into().map_err(|_| Error::TruncatedDatagram)?),
            downstream_mtu: u16::from_be_bytes(bytes[6..8].try_into().map_err(|_| Error::TruncatedDatagram)?),
        })
    }

    fn encode(self, datagram: &mut Vec<u8>) {
        datagram.extend_from_slice(&self.initial_sequence_number.to_be_bytes());
        datagram.extend_from_slice(&self.upstream_mtu.to_be_bytes());
        datagram.extend_from_slice(&self.downstream_mtu.to_be_bytes());
    }
}

#[cfg(feature = "alloc")]
struct SourcePayload<'a> {
    source_sequence: u32,
    data: &'a [u8],
}

#[cfg(feature = "alloc")]
impl<'a> SourcePayload<'a> {
    fn decode(bytes: &'a [u8]) -> Result<Self, Error> {
        let header = bytes.get(..SOURCE_HEADER_SIZE).ok_or(Error::TruncatedDatagram)?;
        let source_sequence = u32::from_be_bytes(header[4..8].try_into().map_err(|_| Error::TruncatedDatagram)?);
        Ok(Self {
            source_sequence,
            data: &bytes[SOURCE_HEADER_SIZE..],
        })
    }
}

#[cfg(feature = "alloc")]
#[derive(Clone, Copy)]
struct V3Header {
    flags: u16,
    log_window_size: u8,
}

#[cfg(feature = "alloc")]
impl V3Header {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let bytes: [u8; V3_HEADER_SIZE] = bytes
            .get(..V3_HEADER_SIZE)
            .ok_or(Error::TruncatedDatagram)?
            .try_into()
            .map_err(|_| Error::TruncatedDatagram)?;
        let header = u16::from_le_bytes(bytes);
        Ok(Self {
            flags: header & 0x0fff,
            log_window_size: u8::try_from(header >> 12).map_err(|_| Error::InvalidFlags)?,
        })
    }

    fn encode(self, datagram: &mut Vec<u8>) {
        let header = self.flags | (u16::from(self.log_window_size) << 12);
        datagram.extend_from_slice(&header.to_le_bytes());
    }
}

#[cfg(feature = "alloc")]
struct V3DataPayload<'a> {
    data_sequence: u16,
    channel_sequence: u16,
    data: &'a [u8],
}

#[cfg(feature = "alloc")]
impl<'a> V3DataPayload<'a> {
    fn decode(bytes: &'a [u8]) -> Result<Self, Error> {
        let header = bytes
            .get(..V3_DATA_HEADER_SIZE + V3_CHANNEL_HEADER_SIZE)
            .ok_or(Error::TruncatedDatagram)?;
        Ok(Self {
            data_sequence: u16::from_le_bytes(
                header[..V3_DATA_HEADER_SIZE]
                    .try_into()
                    .map_err(|_| Error::TruncatedDatagram)?,
            ),
            channel_sequence: u16::from_le_bytes(
                header[V3_DATA_HEADER_SIZE..V3_DATA_HEADER_SIZE + V3_CHANNEL_HEADER_SIZE]
                    .try_into()
                    .map_err(|_| Error::TruncatedDatagram)?,
            ),
            data: &bytes[V3_DATA_HEADER_SIZE + V3_CHANNEL_HEADER_SIZE..],
        })
    }
}

#[cfg(feature = "alloc")]
struct V3AckPayload {
    sequence: u16,
    received_timestamp: u32,
}

#[cfg(feature = "alloc")]
impl V3AckPayload {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let bytes: [u8; V3_ACK_SIZE] = bytes
            .get(..V3_ACK_SIZE)
            .ok_or(Error::TruncatedDatagram)?
            .try_into()
            .map_err(|_| Error::TruncatedDatagram)?;
        let delayed_acks = bytes[6] >> 4;
        if delayed_acks != 0 {
            return Err(Error::UnsupportedRdpUdp2Payload);
        }
        Ok(Self {
            sequence: u16::from_le_bytes(bytes[..2].try_into().map_err(|_| Error::TruncatedDatagram)?),
            received_timestamp: u32::from_le_bytes([bytes[2], bytes[3], bytes[4], 0]),
        })
    }

    fn encode(self, datagram: &mut Vec<u8>) {
        datagram.extend_from_slice(&self.sequence.to_le_bytes());
        let timestamp = self.received_timestamp.to_le_bytes();
        datagram.extend_from_slice(&timestamp[..3]);
        datagram.extend_from_slice(&[0, 0]);
    }
}

#[cfg(feature = "alloc")]
fn encode_v3_wire_packet(mut layout: Vec<u8>) -> Vec<u8> {
    let short_length = if layout.len() < 7 {
        let length = match u8::try_from(layout.len()) {
            Ok(length) => length,
            Err(_) => return layout,
        };
        layout.resize(7, 0);
        length
    } else {
        0
    };
    let prefix = short_length << 5;
    layout.insert(0, prefix);
    layout.swap(0, 7);
    layout
}

#[cfg(feature = "alloc")]
fn decode_v3_wire_packet(datagram: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    if datagram.len() <= 7 {
        return Err(Error::TruncatedDatagram);
    }
    let mut packet = datagram.to_vec();
    packet.swap(0, 7);
    let prefix = packet.remove(0);
    if prefix & 0x01 != 0 {
        return Err(Error::InvalidFlags);
    }
    let packet_type_index = (prefix >> 1) & 0x0f;
    if packet_type_index == 8 {
        return Ok(None);
    }
    if packet_type_index != 0 {
        return Err(Error::InvalidFlags);
    }
    let short_length = prefix >> 5;
    if short_length != 0 {
        let padding = 7usize.saturating_sub(usize::from(short_length));
        if packet.len() < 7 || short_length > 7 || padding > packet.len() {
            return Err(Error::InvalidFlags);
        }
        let layout_len = packet.len() - padding;
        packet.truncate(layout_len);
    }
    Ok(Some(packet))
}

#[cfg(feature = "alloc")]
fn reconstruct_sequence_16(reference: u64, sequence: u16) -> u64 {
    let candidate = (reference & !0xffff) | u64::from(sequence);
    if candidate > reference && candidate - reference > 0x8000 {
        candidate.saturating_sub(0x10000)
    } else if reference > candidate && reference - candidate > 0x8000 {
        candidate.saturating_add(0x10000)
    } else {
        candidate
    }
}

#[cfg(feature = "alloc")]
fn low_sequence(sequence: u64) -> u16 {
    u16::from_le_bytes(low_sequence_bytes(sequence))
}

#[cfg(feature = "alloc")]
fn low_sequence_bytes(sequence: u64) -> [u8; 2] {
    let bytes = sequence.to_le_bytes();
    [bytes[0], bytes[1]]
}

#[cfg(feature = "alloc")]
fn v3_log_window_size(receive_window_size: u16) -> u8 {
    let size = u32::from(receive_window_size);
    u8::try_from(size.next_power_of_two().ilog2()).unwrap_or(15)
}

#[cfg(feature = "alloc")]
fn timestamp_4us(now: Timestamp) -> u32 {
    let ticks = now.elapsed().as_micros() / 4;
    u32::try_from(ticks & 0x00ff_ffff).unwrap_or_default()
}

#[cfg(feature = "alloc")]
fn parse_peer_version(datagram: &[u8], flags: u16) -> Result<ProtocolVersion, Error> {
    let mut offset = COMMON_HEADER_SIZE + SYN_DATA_SIZE;
    if flags & FLAG_CORRELATION_ID != 0 {
        let correlation_id = datagram
            .get(offset..offset + CORRELATION_ID_PAYLOAD_SIZE)
            .ok_or(Error::TruncatedDatagram)?;
        if correlation_id[16..].iter().any(|&byte| byte != 0) {
            return Err(Error::InvalidFlags);
        }
        offset += CORRELATION_ID_PAYLOAD_SIZE;
    }
    if flags & FLAG_SYN_EX == 0 {
        if datagram[offset..].iter().any(|&byte| byte != 0) {
            return Err(Error::InvalidFlags);
        }
        return Ok(ProtocolVersion::V1);
    }
    let bytes: [u8; 4] = datagram
        .get(offset..offset + 4)
        .ok_or(Error::TruncatedDatagram)?
        .try_into()
        .map_err(|_| Error::TruncatedDatagram)?;
    let syn_ex_flags = u16::from_be_bytes(bytes[..2].try_into().map_err(|_| Error::TruncatedDatagram)?);
    if syn_ex_flags & !SYN_EX_VERSION_INFO_VALID != 0 || syn_ex_flags & SYN_EX_VERSION_INFO_VALID == 0 {
        return Err(Error::InvalidFlags);
    }
    let version = ProtocolVersion::from_wire(u16::from_be_bytes(
        bytes[2..].try_into().map_err(|_| Error::TruncatedDatagram)?,
    ))
    .ok_or(Error::UnsupportedVersion)?;
    offset += bytes.len();
    if version == ProtocolVersion::V3 {
        offset = offset.checked_add(32).ok_or(Error::TruncatedDatagram)?;
        let _ = datagram.get(..offset).ok_or(Error::TruncatedDatagram)?;
    }
    if datagram[offset..].iter().any(|&byte| byte != 0) {
        return Err(Error::InvalidFlags);
    }
    Ok(version)
}

#[cfg(feature = "alloc")]
fn validate_client_syn_header(header: Header) -> Result<(), Error> {
    const CLIENT_SYN_FLAGS: u16 = FLAG_SYN | FLAG_CORRELATION_ID | FLAG_SYN_EX;

    if header.flags & (FLAG_FEC | FLAG_SYN_LOSSY) != 0 {
        return Err(Error::UnsupportedTransportMode);
    }
    if header.flags & !CLIENT_SYN_FLAGS != 0 || header.flags & FLAG_SYN == 0 || header.source_ack != u32::MAX {
        return Err(Error::InvalidFlags);
    }
    Ok(())
}

#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum AckVectorState {
    Received,
    NotYetReceived,
}

#[cfg(feature = "alloc")]
impl AckVectorState {
    fn encode(self, length: u8) -> u8 {
        let state = match self {
            Self::Received => 0,
            Self::NotYetReceived => 3,
        };
        (state << 6) | length
    }

    fn decode(element: u8) -> Result<(Self, u8), Error> {
        let length = element & 0x3f;
        if length == 0 {
            return Err(Error::InvalidFlags);
        }
        let state = match element >> 6 {
            0 => Self::Received,
            3 => Self::NotYetReceived,
            _ => return Err(Error::InvalidFlags),
        };
        Ok((state, length))
    }
}

#[cfg(feature = "alloc")]
fn decode_ack_vector(bytes: &[u8], maximum_states: usize) -> Result<(Vec<AckVectorState>, usize), Error> {
    let encoded_len = bytes.get(..2).ok_or(Error::TruncatedDatagram)?;
    let encoded_len = usize::from(u16::from_be_bytes(
        encoded_len.try_into().map_err(|_| Error::TruncatedDatagram)?,
    ));
    if encoded_len > MAX_ACK_VECTOR_SIZE {
        return Err(Error::InvalidFlags);
    }
    let padding = ack_vector_padding(encoded_len);
    let size = ACK_VECTOR_HEADER_SIZE + encoded_len + padding;
    if bytes.len() < size {
        return Err(Error::TruncatedDatagram);
    }
    if bytes[ACK_VECTOR_HEADER_SIZE + encoded_len..size]
        .iter()
        .any(|&byte| byte != 0)
    {
        return Err(Error::InvalidFlags);
    }

    let mut states = Vec::new();
    for &element in &bytes[ACK_VECTOR_HEADER_SIZE..ACK_VECTOR_HEADER_SIZE + encoded_len] {
        let (state, length) = AckVectorState::decode(element)?;
        let length = usize::from(length);
        if states.len().saturating_add(length) > maximum_states {
            return Err(Error::InvalidFlags);
        }
        states.extend(core::iter::repeat_n(state, length));
    }
    if states.last().is_some_and(|state| *state != AckVectorState::Received) {
        return Err(Error::InvalidFlags);
    }
    Ok((states, size))
}

#[cfg(feature = "alloc")]
fn ack_vector_padding(encoded_len: usize) -> usize {
    (4 - ((ACK_VECTOR_HEADER_SIZE + encoded_len) % 4)) % 4
}

#[cfg(feature = "alloc")]
fn validate_fec_payload(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() <= FEC_HEADER_SIZE {
        return Err(Error::TruncatedDatagram);
    }
    if bytes[10..FEC_HEADER_SIZE].iter().any(|&byte| byte != 0) {
        return Err(Error::InvalidFlags);
    }
    Ok(())
}

#[cfg(feature = "alloc")]
fn sequence_after(left: u32, right: u32) -> bool {
    left != right && left.wrapping_sub(right) < (1 << 31)
}

#[cfg(feature = "alloc")]
fn sequence_after_or_equal(left: u32, right: u32) -> bool {
    left == right || sequence_after(left, right)
}

#[cfg(feature = "alloc")]
fn sequence_before_or_equal(left: u32, right: u32) -> bool {
    left == right || sequence_after(right, left)
}

#[cfg(feature = "alloc")]
fn sequence_in_receive_window(sequence: u32, first: u32, window: u16) -> bool {
    window != 0 && sequence.wrapping_sub(first) < u32::from(window)
}

#[cfg(feature = "alloc")]
fn sequence_in_window(sequence: u32, cumulative_ack: u32, window: u16) -> bool {
    sequence_after(sequence, cumulative_ack) && sequence.wrapping_sub(cumulative_ack) <= u32::from(window)
}

#[cfg(all(test, feature = "alloc"))]
#[expect(clippy::unwrap_used, reason = "test fixtures use infallible protocol setup")]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            initial_sequence_number: 7,
            max_version: ProtocolVersion::V2,
            receive_window_size: 8,
            mtu: 1232,
            max_retransmits: 3,
            max_reorder_buffer: 7,
            max_delivered_messages: 8,
        }
    }

    fn server_syn_ack_for(version: ProtocolVersion) -> Vec<u8> {
        let mut datagram = Vec::new();
        Header {
            source_ack: 7,
            receive_window_size: 1024,
            flags: FLAG_SYN | FLAG_ACK | FLAG_SYN_EX,
        }
        .encode(&mut datagram);
        SynData {
            initial_sequence_number: 42,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }
        .encode(&mut datagram);
        datagram.extend_from_slice(&SYN_EX_VERSION_INFO_VALID.to_be_bytes());
        datagram.extend_from_slice(&version.as_wire().to_be_bytes());
        datagram
    }

    fn server_syn_ack() -> Vec<u8> {
        server_syn_ack_for(ProtocolVersion::V2)
    }

    fn source_datagram(source_ack: u32, source_sequence: u32) -> Vec<u8> {
        let mut datagram = Vec::new();
        Header {
            source_ack,
            receive_window_size: 8,
            flags: FLAG_ACK | FLAG_DATA,
        }
        .encode(&mut datagram);
        datagram.extend_from_slice(&[0, 0, 0, 0]);
        datagram.extend_from_slice(&source_sequence.to_be_bytes());
        datagram.extend_from_slice(&source_sequence.to_be_bytes());
        datagram.extend_from_slice(b"data");
        datagram
    }

    fn open_pair(now: Timestamp, server_config: Config) -> (ReliableUdp, ReliableUdp) {
        let mut client = ReliableUdp::new(config()).unwrap();
        let client_syn = client.start(now).unwrap();
        let (mut server, server_syn_ack) = ReliableUdp::accept(server_config, now, &client_syn.0).unwrap();
        let client_ack = client.handle_datagram(now, &server_syn_ack.0).unwrap();
        server.handle_datagram(now, &client_ack[0].0).unwrap();
        (client, server)
    }

    #[test]
    fn handshake_then_orders_reordered_source_packets() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut transport = ReliableUdp::new(config()).unwrap();
        let syn = transport.start(now).unwrap();
        assert_eq!(syn.0[6..8], (FLAG_SYN | FLAG_SYN_EX).to_be_bytes());
        assert_eq!(transport.handle_datagram(now, &server_syn_ack()).unwrap().len(), 1);
        assert_eq!(
            transport.state(),
            State::Open {
                version: ProtocolVersion::V2
            }
        );

        let mut packet_43 = Vec::new();
        Header {
            source_ack: 7,
            receive_window_size: 8,
            flags: FLAG_ACK | FLAG_DATA,
        }
        .encode(&mut packet_43);
        packet_43.extend_from_slice(&[0, 0, 0, 0]);
        packet_43.extend_from_slice(&44u32.to_be_bytes());
        packet_43.extend_from_slice(&44u32.to_be_bytes());
        packet_43.extend_from_slice(b"second");
        transport.handle_datagram(now, &packet_43).unwrap();

        let mut packet_42 = Vec::new();
        Header {
            source_ack: 7,
            receive_window_size: 8,
            flags: FLAG_ACK | FLAG_DATA,
        }
        .encode(&mut packet_42);
        packet_42.extend_from_slice(&[0, 0, 0, 0]);
        packet_42.extend_from_slice(&43u32.to_be_bytes());
        packet_42.extend_from_slice(&43u32.to_be_bytes());
        packet_42.extend_from_slice(b"first");
        transport.handle_datagram(now, &packet_42).unwrap();

        assert_eq!(transport.take_received(), Some(b"first".to_vec()));
        assert_eq!(transport.take_received(), Some(b"second".to_vec()));
    }

    #[test]
    fn retransmits_syn_until_budget_is_exhausted() {
        let mut transport = ReliableUdp::new(config()).unwrap();
        let now = Timestamp::from_elapsed(Duration::ZERO);
        transport.start(now).unwrap();
        for attempt in 1..=3 {
            let deadline = transport.next_deadline().unwrap();
            assert_eq!(
                transport.handle_timeout(deadline).unwrap().len(),
                1,
                "attempt {attempt}"
            );
        }
        let deadline = transport.next_deadline().unwrap();
        assert_eq!(transport.handle_timeout(deadline), Err(Error::RetransmitLimitReached));
        assert_eq!(transport.state(), State::Failed);
    }

    #[test]
    fn server_accepts_syn_and_delivers_reliable_data_in_both_directions() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client = ReliableUdp::new(config()).unwrap();
        let client_syn = client.start(now).unwrap();

        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut server, server_syn_ack) = ReliableUdp::accept(server_config, now, &client_syn.0).unwrap();
        assert_eq!(
            server.state(),
            State::SynReceived {
                version: ProtocolVersion::V2
            }
        );

        let client_ack = client.handle_datagram(now, &server_syn_ack.0).unwrap();
        assert_eq!(client_ack.len(), 1);
        assert_eq!(
            client.state(),
            State::Open {
                version: ProtocolVersion::V2
            }
        );
        assert!(server.handle_datagram(now, &client_ack[0].0).unwrap().is_empty());
        assert_eq!(
            server.state(),
            State::Open {
                version: ProtocolVersion::V2
            }
        );

        let client_data = client.send(now, b"from client".to_vec()).unwrap();
        let server_ack = server.handle_datagram(now, &client_data.0).unwrap();
        assert_eq!(server.take_received(), Some(b"from client".to_vec()));
        client.handle_datagram(now, &server_ack[0].0).unwrap();

        let server_data = server.send(now, b"from server".to_vec()).unwrap();
        let client_ack = client.handle_datagram(now, &server_data.0).unwrap();
        assert_eq!(client.take_received(), Some(b"from server".to_vec()));
        server.handle_datagram(now, &client_ack[0].0).unwrap();
        assert_eq!(
            server.next_deadline(),
            Some(Timestamp::from_elapsed(KEEPALIVE_INTERVAL))
        );
    }

    #[test]
    fn server_retransmits_syn_ack_until_client_acknowledges_it() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client = ReliableUdp::new(config()).unwrap();
        let client_syn = client.start(now).unwrap();

        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut server, syn_ack) = ReliableUdp::accept(server_config, now, &client_syn.0).unwrap();
        let deadline = server.next_deadline().unwrap();
        assert_eq!(server.handle_timeout(deadline).unwrap(), vec![syn_ack]);
    }

    #[test]
    fn server_rejects_a_syn_with_a_non_sentinel_acknowledgement() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client = ReliableUdp::new(config()).unwrap();
        let mut client_syn = client.start(now).unwrap().0;
        client_syn[..4].copy_from_slice(&0u32.to_be_bytes());

        assert!(matches!(
            ReliableUdp::accept(config(), now, &client_syn),
            Err(Error::InvalidFlags)
        ));
    }

    #[test]
    fn rejects_unrelated_syn_and_syn_ack_flags() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client = ReliableUdp::new(config()).unwrap();
        let mut client_syn = client.start(now).unwrap().0;
        let client_syn_flags = u16::from_be_bytes(client_syn[6..8].try_into().unwrap());
        client_syn[6..8].copy_from_slice(&(client_syn_flags | FLAG_DATA).to_be_bytes());
        assert!(matches!(
            ReliableUdp::accept(config(), now, &client_syn),
            Err(Error::InvalidFlags)
        ));

        let mut syn_ack = server_syn_ack();
        let syn_ack_flags = u16::from_be_bytes(syn_ack[6..8].try_into().unwrap());
        syn_ack[6..8].copy_from_slice(&(syn_ack_flags | FLAG_DATA).to_be_bytes());
        assert_eq!(client.handle_datagram(now, &syn_ack), Err(Error::InvalidFlags));
    }

    #[test]
    fn accepts_a_correlation_id_before_the_synex_payload() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut syn = Vec::new();
        Header {
            source_ack: u32::MAX,
            receive_window_size: 8,
            flags: FLAG_SYN | FLAG_CORRELATION_ID | FLAG_SYN_EX,
        }
        .encode(&mut syn);
        SynData {
            initial_sequence_number: 7,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }
        .encode(&mut syn);
        syn.extend_from_slice(&[1; 16]);
        syn.extend_from_slice(&[0; 16]);
        syn.extend_from_slice(&SYN_EX_VERSION_INFO_VALID.to_be_bytes());
        syn.extend_from_slice(&ProtocolVersion::V2.as_wire().to_be_bytes());
        syn.resize(1232, 0);

        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (server, _) = ReliableUdp::accept(server_config, now, &syn).unwrap();
        assert_eq!(
            server.state(),
            State::SynReceived {
                version: ProtocolVersion::V2
            }
        );

        syn[COMMON_HEADER_SIZE + SYN_DATA_SIZE + 16] = 1;
        assert!(matches!(
            ReliableUdp::accept(server_config, now, &syn),
            Err(Error::InvalidFlags)
        ));
    }

    #[test]
    fn rejects_open_datagrams_without_an_ack_vector() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (_, mut server) = open_pair(now, server_config);

        let mut datagram = Vec::new();
        Header {
            source_ack: 42,
            receive_window_size: 8,
            flags: 0,
        }
        .encode(&mut datagram);
        assert_eq!(server.handle_datagram(now, &datagram), Err(Error::InvalidFlags));
    }

    #[test]
    fn server_keeps_the_syn_ack_retry_state_after_a_malformed_final_ack() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client = ReliableUdp::new(config()).unwrap();
        let client_syn = client.start(now).unwrap();

        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut server, syn_ack) = ReliableUdp::accept(server_config, now, &client_syn.0).unwrap();
        let mut client_ack = client.handle_datagram(now, &syn_ack.0).unwrap().remove(0).0;
        client_ack.pop();

        assert_eq!(server.handle_datagram(now, &client_ack), Err(Error::TruncatedDatagram));
        assert_eq!(
            server.state(),
            State::SynReceived {
                version: ProtocolVersion::V2
            }
        );
        assert_eq!(
            server.handle_timeout(server.next_deadline().unwrap()).unwrap(),
            vec![syn_ack]
        );
    }

    #[test]
    fn retransmitted_syn_ack_is_acknowledged_after_client_opens() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client = ReliableUdp::new(config()).unwrap();
        let client_syn = client.start(now).unwrap();

        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut server, syn_ack) = ReliableUdp::accept(server_config, now, &client_syn.0).unwrap();

        let lost_ack = client.handle_datagram(now, &syn_ack.0).unwrap();
        assert_eq!(
            client.state(),
            State::Open {
                version: ProtocolVersion::V2
            }
        );

        let recovered_ack = client.handle_datagram(now, &syn_ack.0).unwrap();
        assert_eq!(recovered_ack, lost_ack);
        assert!(server.handle_datagram(now, &recovered_ack[0].0).unwrap().is_empty());
        assert_eq!(
            server.state(),
            State::Open {
                version: ProtocolVersion::V2
            }
        );
    }

    #[test]
    fn server_ignores_a_reordered_client_syn_after_opening() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client = ReliableUdp::new(config()).unwrap();
        let client_syn = client.start(now).unwrap();

        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut server, server_syn_ack) = ReliableUdp::accept(server_config, now, &client_syn.0).unwrap();
        let client_ack = client.handle_datagram(now, &server_syn_ack.0).unwrap();
        server.handle_datagram(now, &client_ack[0].0).unwrap();

        assert_eq!(
            server.state(),
            State::Open {
                version: ProtocolVersion::V2
            }
        );
        assert!(server.handle_datagram(now, &client_syn.0).unwrap().is_empty());
        assert_eq!(
            server.state(),
            State::Open {
                version: ProtocolVersion::V2
            }
        );
    }

    #[test]
    fn retransmitted_source_is_acknowledged_without_redelivery() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut client, mut server) = open_pair(now, server_config);

        let data = client.send(now, b"data".to_vec()).unwrap();
        let _lost_ack = server.handle_datagram(now, &data.0).unwrap();
        assert_eq!(server.take_received(), Some(b"data".to_vec()));

        let retransmission = client.handle_timeout(client.next_deadline().unwrap()).unwrap();
        assert_eq!(retransmission.len(), 1);
        let recovered_ack = server.handle_datagram(now, &retransmission[0].0).unwrap();
        assert_eq!(recovered_ack.len(), 1);
        assert_eq!(Header::decode(&recovered_ack[0].0).unwrap().flags & FLAG_ACK, FLAG_ACK);
        assert_eq!(server.take_received(), None);

        client.handle_datagram(now, &recovered_ack[0].0).unwrap();
        assert!(client.pending.is_empty());
    }

    #[test]
    fn sets_congestion_notification_only_after_three_later_packets_are_received() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (_, mut server) = open_pair(now, server_config);

        for source_sequence in [11, 12] {
            let response = server
                .handle_datagram(now, &source_datagram(42, source_sequence))
                .unwrap();
            assert_eq!(Header::decode(&response[0].0).unwrap().flags & FLAG_CN, 0);
        }

        let response = server.handle_datagram(now, &source_datagram(42, 13)).unwrap();
        assert_ne!(Header::decode(&response[0].0).unwrap().flags & FLAG_CN, 0);
    }

    #[test]
    fn retransmission_timeout_does_not_shrink_after_a_smaller_rtt_sample() {
        let start = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut client, mut server) = open_pair(start, server_config);

        let initial = client.send(start, b"initial".to_vec()).unwrap();
        let initial_ack = server.handle_datagram(start, &initial.0).unwrap();
        client
            .handle_datagram(Timestamp::from_elapsed(Duration::from_millis(250)), &initial_ack[0].0)
            .unwrap();

        let sent_at = Timestamp::from_elapsed(Duration::from_millis(250));
        client.send(sent_at, b"lost".to_vec()).unwrap();
        let later = client.send(sent_at, b"later".to_vec()).unwrap();
        let later_ack = server.handle_datagram(sent_at, &later.0).unwrap();
        client
            .handle_datagram(Timestamp::from_elapsed(Duration::from_millis(300)), &later_ack[0].0)
            .unwrap();

        client
            .handle_timeout(Timestamp::from_elapsed(Duration::from_millis(750)))
            .unwrap();

        assert_eq!(
            client.pending[&9].deadline,
            Timestamp::from_elapsed(Duration::from_millis(1250))
        );
    }

    #[test]
    fn rejects_ack_vectors_larger_than_the_peer_window() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut client, _) = open_pair(now, server_config);

        let mut acknowledgment = Vec::new();
        Header {
            source_ack: 7,
            receive_window_size: 8,
            flags: FLAG_ACK,
        }
        .encode(&mut acknowledgment);
        acknowledgment.extend_from_slice(&[0, 1, AckVectorState::Received.encode(9), 0]);

        assert_eq!(client.handle_datagram(now, &acknowledgment), Err(Error::InvalidFlags));
    }

    #[test]
    fn rejects_unsupported_version_3_configuration() {
        let mut config = config();
        config.max_version = ProtocolVersion::V3;
        assert!(matches!(ReliableUdp::new(config), Err(Error::UnsupportedVersion)));
    }

    #[test]
    fn rejects_version_3_before_emitting_a_syn() {
        let mut config = config();
        config.max_version = ProtocolVersion::V3;
        assert!(matches!(ReliableUdp::new(config), Err(Error::UnsupportedVersion)));
    }

    #[test]
    fn reconstructs_16_bit_sequences_across_a_wrap() {
        assert_eq!(reconstruct_sequence_16(0x1234_ff68, 0xff78), 0x1234_ff78);
        assert_eq!(reconstruct_sequence_16(0x1234_ff68, 0x0003), 0x1235_0003);
    }

    #[test]
    fn v3_short_packet_prefix_is_placed_in_wire_byte_seven() {
        let mut layout = Vec::new();
        V3Header {
            flags: V3_FLAG_DATA,
            log_window_size: 10,
        }
        .encode(&mut layout);
        layout.extend_from_slice(&7u16.to_le_bytes());
        layout.extend_from_slice(&7u16.to_le_bytes());

        let wire = encode_v3_wire_packet(layout.clone());
        assert_eq!(wire.len(), 8);
        assert_eq!(wire[7], 6 << 5);
        assert_eq!(decode_v3_wire_packet(&wire).unwrap(), Some(layout));
    }

    #[test]
    fn rejects_v3_ack_vectors_by_rejecting_version_3() {
        let mut config = config();
        config.max_version = ProtocolVersion::V3;
        assert!(matches!(ReliableUdp::new(config), Err(Error::UnsupportedVersion)));
    }

    #[test]
    fn pads_syn_and_syn_ack_to_the_negotiated_mtu() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client = ReliableUdp::new(config()).unwrap();
        let syn = client.start(now).unwrap();
        assert_eq!(syn.0.len(), usize::from(config().mtu));
        assert!(
            syn.0[COMMON_HEADER_SIZE + SYN_DATA_SIZE + 4..]
                .iter()
                .all(|&byte| byte == 0)
        );

        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (_, syn_ack) = ReliableUdp::accept(server_config, now, &syn.0).unwrap();
        assert_eq!(syn_ack.0.len(), usize::from(config().mtu));
        assert!(
            syn_ack.0[COMMON_HEADER_SIZE + SYN_DATA_SIZE + 4..]
                .iter()
                .all(|&byte| byte == 0)
        );
    }

    #[test]
    fn recovers_a_lost_source_packet_from_an_ack_vector() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut client, mut server) = open_pair(now, server_config);

        client.send(now, b"first".to_vec()).unwrap();
        let second = client.send(now, b"second".to_vec()).unwrap();
        let acknowledgment = server.handle_datagram(now, &second.0).unwrap();
        client.handle_datagram(now, &acknowledgment[0].0).unwrap();

        let retransmission = client
            .handle_timeout(Timestamp::from_elapsed(Duration::from_millis(300)))
            .unwrap();
        assert_eq!(retransmission.len(), 1);
        assert_eq!(u32::from_be_bytes(retransmission[0].0[12..16].try_into().unwrap()), 10);
        assert_ne!(
            u16::from_be_bytes(retransmission[0].0[6..8].try_into().unwrap()) & FLAG_CWR,
            0
        );

        let final_acknowledgment = server.handle_datagram(now, &retransmission[0].0).unwrap();
        client.handle_datagram(now, &final_acknowledgment[0].0).unwrap();
        assert_eq!(server.take_received(), Some(b"first".to_vec()));
        assert_eq!(server.take_received(), Some(b"second".to_vec()));
        assert!(client.next_deadline().is_some());
    }

    #[test]
    fn does_not_fast_retransmit_the_same_source_packet_twice() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut client, mut server) = open_pair(now, server_config);

        let _first = client.send(now, b"first".to_vec()).unwrap();
        let second = client.send(now, b"second".to_vec()).unwrap();
        let third = client.send(now, b"third".to_vec()).unwrap();
        let fourth = client.send(now, b"fourth".to_vec()).unwrap();
        server.handle_datagram(now, &second.0).unwrap();
        server.handle_datagram(now, &third.0).unwrap();
        let acknowledgement = server.handle_datagram(now, &fourth.0).unwrap().remove(0);

        assert_eq!(client.handle_datagram(now, &acknowledgement.0).unwrap().len(), 1);
        assert!(client.handle_datagram(now, &acknowledgement.0).unwrap().is_empty());
    }

    #[test]
    fn applies_and_emits_ack_of_acks_reset_positions() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (_, mut server) = open_pair(now, server_config);

        for source_sequence in [9, 10, 11] {
            server
                .handle_datagram(now, &source_datagram(42, source_sequence))
                .unwrap();
        }

        let mut reset = Vec::new();
        Header {
            source_ack: 42,
            receive_window_size: 8,
            flags: FLAG_ACK | FLAG_ACK_OF_ACKS,
        }
        .encode(&mut reset);
        reset.extend_from_slice(&[0, 0, 0, 0]);
        reset.extend_from_slice(&10u32.to_be_bytes());
        server.handle_datagram(now, &reset).unwrap();
        let source = server.send(now, b"source".to_vec()).unwrap();
        assert_eq!(u16::from_be_bytes(source.0[8..10].try_into().unwrap()), 1);
        assert_eq!(source.0[10], AckVectorState::Received.encode(1));

        let mut emission_server_config = config();
        emission_server_config.initial_sequence_number = 42;
        let (mut emission_client, mut emission_server) = open_pair(now, emission_server_config);
        let client_source = emission_client.send(now, b"client".to_vec()).unwrap();
        let acknowledgement = emission_server
            .handle_datagram(now, &client_source.0)
            .unwrap()
            .remove(0);
        for _ in 0..20 {
            emission_client.handle_datagram(now, &acknowledgement.0).unwrap();
        }
        let source = emission_client.send(now, b"next".to_vec()).unwrap();
        assert_ne!(Header::decode(&source.0).unwrap().flags & FLAG_ACK_OF_ACKS, 0);
        assert_eq!(u32::from_be_bytes(source.0[12..16].try_into().unwrap()), 8);
    }

    #[test]
    fn transfers_a_full_negotiated_source_payload() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client_config = config();
        client_config.mtu = 1200;
        let mut server_config = client_config;
        server_config.initial_sequence_number = 42;
        let (mut client, mut server) = open_pair(now, server_config);

        let payload = vec![0xA5; 1200];
        let source = client.send(now, payload.clone()).unwrap();
        assert!(source.0.len() > 1200);

        server.handle_datagram(now, &source.0).unwrap();
        assert_eq!(server.take_received(), Some(payload));
    }

    #[test]
    fn ignores_a_source_payload_larger_than_the_negotiated_mtu() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut client_config = config();
        client_config.mtu = 1200;
        let mut server_config = client_config;
        server_config.initial_sequence_number = 42;
        let (_, mut server) = open_pair(now, server_config);

        let mut datagram = Vec::new();
        Header {
            source_ack: 7,
            receive_window_size: 8,
            flags: FLAG_ACK | FLAG_DATA,
        }
        .encode(&mut datagram);
        datagram.extend_from_slice(&[0, 0, 0, 0]);
        datagram.extend_from_slice(&8u32.to_be_bytes());
        datagram.extend_from_slice(&8u32.to_be_bytes());
        datagram.resize(datagram.len() + 1201, 0);

        assert!(server.handle_datagram(now, &datagram).unwrap().is_empty());
        assert_eq!(server.take_received(), None);
    }

    #[test]
    fn enforces_the_peer_receive_window() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        server_config.receive_window_size = 1;
        server_config.max_reorder_buffer = 0;
        server_config.max_delivered_messages = 1;
        let (mut client, _) = open_pair(now, server_config);

        client.send(now, b"first".to_vec()).unwrap();
        assert_eq!(client.send(now, b"second".to_vec()), Err(Error::SendWindowFull));
    }

    #[test]
    fn discards_source_packets_outside_the_advertised_window() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (_, mut server) = open_pair(now, server_config);

        let mut datagram = Vec::new();
        Header {
            source_ack: 7,
            receive_window_size: 8,
            flags: FLAG_ACK | FLAG_DATA,
        }
        .encode(&mut datagram);
        datagram.extend_from_slice(&[0, 0, 0, 0]);
        datagram.extend_from_slice(&8u32.to_be_bytes());
        datagram.extend_from_slice(&16u32.to_be_bytes());
        datagram.extend_from_slice(b"outside window");

        assert!(server.handle_datagram(now, &datagram).unwrap().is_empty());
        assert_eq!(server.take_received(), None);
    }

    #[test]
    fn accepts_optional_fec_packets_without_acknowledging_them() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (_, mut server) = open_pair(now, server_config);

        let mut datagram = Vec::new();
        Header {
            source_ack: 7,
            receive_window_size: 8,
            flags: FLAG_ACK | FLAG_DATA | FLAG_FEC,
        }
        .encode(&mut datagram);
        datagram.extend_from_slice(&[0, 0, 0, 0]);
        datagram.extend_from_slice(&8u32.to_be_bytes());
        datagram.extend_from_slice(&8u32.to_be_bytes());
        datagram.extend_from_slice(&[0, 0, 0, 0]);
        datagram.extend_from_slice(&[0xAA]);

        assert!(server.handle_datagram(now, &datagram).unwrap().is_empty());
    }

    #[test]
    fn sends_a_congestion_window_reset_after_a_notification() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut client, _) = open_pair(now, server_config);

        let mut notification = Vec::new();
        Header {
            source_ack: 7,
            receive_window_size: 8,
            flags: FLAG_ACK | FLAG_CN,
        }
        .encode(&mut notification);
        notification.extend_from_slice(&[0, 0, 0, 0]);
        client.handle_datagram(now, &notification).unwrap();

        let source = client.send(now, b"data".to_vec()).unwrap();
        assert_ne!(u16::from_be_bytes(source.0[6..8].try_into().unwrap()) & FLAG_CWR, 0);
    }

    #[test]
    fn closes_when_no_peer_datagram_arrives_within_the_keepalive_timeout() {
        let now = Timestamp::from_elapsed(Duration::ZERO);
        let mut server_config = config();
        server_config.initial_sequence_number = 42;
        let (mut client, _) = open_pair(now, server_config);

        assert_eq!(
            client.handle_timeout(Timestamp::from_elapsed(PEER_TIMEOUT)),
            Err(Error::PeerTimedOut)
        );
        assert_eq!(client.state(), State::Failed);
    }
}
