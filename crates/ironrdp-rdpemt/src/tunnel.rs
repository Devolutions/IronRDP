//! RDPEMT tunnel state machine.
//!
//! Sans-I/O: operates on decrypted PDU bytes (after TLS) and produces
//! plaintext PDU bytes (for TLS encryption). Never touches the network.
//!
//! # Lifecycle
//!
//! ```text
//! Client: Created ──(poll_pdu → CreateRequest)──→ AwaitingResponse
//!         AwaitingResponse ──(CreateResponse S_OK)──→ Established
//!         AwaitingResponse ──(CreateResponse !S_OK)──→ Failed
//!
//! Server: Created ──(CreateRequest, cookie match)──→ Established
//!         Created ──(CreateRequest, no match)──→ Failed
//!
//! Either: Established ──(send_data / handle Data PDU)──→ bidirectional data
//! ```

use alloc::collections::VecDeque;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use crate::error::{RdpemtError, RdpemtErrorExt as _};
use crate::pdu::create_request::SECURITY_COOKIE_LEN;
use crate::pdu::{TunnelCreateRequest, TunnelCreateResponse, TunnelData, TunnelPdu};

// ════════════════════════════════════════════════════════════════════
// Public types
// ════════════════════════════════════════════════════════════════════

/// Which role this tunnel endpoint plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Initiated the tunnel (sends CreateRequest).
    Client,
    /// Accepts the tunnel (validates CreateRequest, sends CreateResponse).
    Server,
}

/// Events produced by the tunnel for the application layer.
///
/// Retrieved by calling `poll_event()` after `handle_pdu()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelEvent {
    /// The tunnel has been established (CreateResponse with S_OK).
    Established,
    /// Higher-layer data received through the tunnel.
    Data(Vec<u8>),
    /// The tunnel creation failed (non-recoverable).
    Failed {
        /// The HRESULT from the CreateResponse, or 0xFFFFFFFF for server-side
        /// cookie mismatch.
        hr_response: u32,
    },
}

/// Configuration for creating a tunnel.
///
/// These values come from the Initiate Multitransport Request PDU
/// that the server sends over the main TCP connection
/// (MS-RDPBCGR Section 2.2.15.1).
#[derive(Debug, Clone)]
pub struct TunnelConfig {
    /// The request ID from the Initiate Multitransport Request PDU.
    pub request_id: u32,
    /// The 16-byte security cookie from the Initiate Multitransport Request PDU.
    pub security_cookie: [u8; SECURITY_COOKIE_LEN],
}

// ════════════════════════════════════════════════════════════════════
// Internal state
// ════════════════════════════════════════════════════════════════════

/// Tunnel lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TunnelState {
    /// Initial state. Client will produce CreateRequest; server awaits it.
    Created,
    /// Client has sent CreateRequest, awaiting CreateResponse.
    AwaitingResponse,
    /// Tunnel is active: Data PDUs can flow both directions.
    Established,
    /// Tunnel creation failed (non-recoverable).
    Failed,
}

// ════════════════════════════════════════════════════════════════════
// RdpemtTunnel
// ════════════════════════════════════════════════════════════════════

/// RDPEMT tunnel state machine.
///
/// Sans-I/O: operates on decrypted PDU bytes (post-TLS) and produces
/// plaintext PDU bytes (pre-TLS). The async driver handles
/// TLS encryption/decryption and RDPEUDP2 transport.
///
/// # Client usage
///
/// ```ignore
/// let mut tunnel = RdpemtTunnel::client(config);
///
/// // 1. Send the CreateRequest
/// let create_req = tunnel.poll_pdu().expect("CreateRequest");
/// tls_send(&create_req);
///
/// // 2. Receive CreateResponse
/// let response_bytes = tls_recv();
/// tunnel.handle_pdu(&response_bytes)?;
/// while let Some(event) = tunnel.poll_event() {
///     match event {
///         TunnelEvent::Established => { /* ready */ }
///         TunnelEvent::Failed { hr_response } => { /* disconnect */ }
///         _ => {}
///     }
/// }
///
/// // 3. Exchange data
/// tunnel.send_data(b"hello")?;
/// let data_pdu = tunnel.poll_pdu().expect("Data PDU");
/// tls_send(&data_pdu);
/// ```
pub struct RdpemtTunnel {
    side: Side,
    state: TunnelState,
    config: TunnelConfig,
    /// PDUs waiting to be sent (encoded wire bytes).
    outgoing: VecDeque<Vec<u8>>,
    /// Events waiting to be delivered to the application.
    events: VecDeque<TunnelEvent>,
}

impl RdpemtTunnel {
    /// Create a client-side tunnel.
    ///
    /// Immediately enqueues the CreateRequest PDU. Call `poll_pdu()` to
    /// retrieve it, then send through TLS.
    pub fn client(config: TunnelConfig) -> Self {
        let mut tunnel = Self {
            side: Side::Client,
            state: TunnelState::Created,
            config,
            outgoing: VecDeque::new(),
            events: VecDeque::new(),
        };
        tunnel.enqueue_create_request();
        tunnel
    }

    /// Create a server-side tunnel.
    ///
    /// The tunnel waits for a CreateRequest from the client. The config
    /// provides the expected request_id + security_cookie for validation.
    pub fn server(config: TunnelConfig) -> Self {
        Self {
            side: Side::Server,
            state: TunnelState::Created,
            config,
            outgoing: VecDeque::new(),
            events: VecDeque::new(),
        }
    }

    /// Process a received (already TLS-decrypted) PDU.
    ///
    /// After calling this, use `poll_event()` to retrieve any events
    /// and `poll_pdu()` to retrieve any response PDUs.
    pub fn handle_pdu(&mut self, data: &[u8]) -> Result<(), RdpemtError> {
        let pdu: TunnelPdu = ironrdp_core::decode(data).map_err(RdpemtError::decode)?;

        match (&self.state, &self.side, pdu) {
            // Client in AwaitingResponse receives CreateResponse
            (TunnelState::AwaitingResponse, Side::Client, TunnelPdu::CreateResponse(resp)) => {
                if resp.is_success() {
                    self.state = TunnelState::Established;
                    self.events.push_back(TunnelEvent::Established);
                } else {
                    self.state = TunnelState::Failed;
                    self.events.push_back(TunnelEvent::Failed {
                        hr_response: resp.hr_response,
                    });
                }
            }

            // Server in Created receives CreateRequest
            (TunnelState::Created, Side::Server, TunnelPdu::CreateRequest(req)) => {
                if req.request_id == self.config.request_id && req.security_cookie == self.config.security_cookie {
                    self.state = TunnelState::Established;
                    self.enqueue_create_response(TunnelCreateResponse::S_OK);
                    self.events.push_back(TunnelEvent::Established);
                } else {
                    self.state = TunnelState::Failed;
                    // E_ACCESSDENIED
                    let hr = 0x80070005u32;
                    self.enqueue_create_response(hr);
                    self.events.push_back(TunnelEvent::Failed { hr_response: hr });
                }
            }

            // Either side in Established receives Data
            (TunnelState::Established, _, TunnelPdu::Data(data_pdu)) => {
                self.events.push_back(TunnelEvent::Data(data_pdu.higher_layer_data));
            }

            // Any other combination is a state violation
            _ => return Err(RdpemtError::invalid_state("handle tunnel PDU")),
        }

        Ok(())
    }

    /// Retrieve the next outgoing PDU to send (encoded, needs TLS encryption).
    ///
    /// Returns `None` when no PDU is pending. Call in a loop until `None`.
    pub fn poll_pdu(&mut self) -> Option<Vec<u8>> {
        self.outgoing.pop_front()
    }

    /// Retrieve the next event for the application layer.
    ///
    /// Returns `None` when no events are pending. Call in a loop until `None`.
    pub fn poll_event(&mut self) -> Option<TunnelEvent> {
        self.events.pop_front()
    }

    /// Queue application data for transmission as a Data PDU.
    ///
    /// The data is wrapped in a TunnelData PDU and enqueued for `poll_pdu()`.
    /// Returns an error if the tunnel is not in the Established state.
    pub fn send_data(&mut self, data: &[u8]) -> Result<(), RdpemtError> {
        if self.state != TunnelState::Established {
            return Err(RdpemtError::invalid_state("send tunnel data"));
        }

        let pdu = TunnelData {
            sub_headers: Vec::new(),
            higher_layer_data: data.to_vec(),
        };

        let encoded = ironrdp_core::encode_vec(&pdu).map_err(RdpemtError::encode)?;
        self.outgoing.push_back(encoded);

        Ok(())
    }

    /// Whether the tunnel is established and ready for data transfer.
    pub fn is_established(&self) -> bool {
        self.state == TunnelState::Established
    }

    /// Whether the tunnel creation has failed.
    pub fn is_failed(&self) -> bool {
        self.state == TunnelState::Failed
    }

    /// Which role this endpoint plays.
    pub fn side(&self) -> Side {
        self.side
    }

    // ── Internal helpers ──

    fn enqueue_create_request(&mut self) {
        let pdu = TunnelCreateRequest {
            request_id: self.config.request_id,
            security_cookie: self.config.security_cookie,
        };

        let encoded = ironrdp_core::encode_vec(&pdu).expect("CreateRequest encoding is infallible for valid inputs");
        self.outgoing.push_back(encoded);
        self.state = TunnelState::AwaitingResponse;
    }

    fn enqueue_create_response(&mut self, hr_response: u32) {
        let pdu = TunnelCreateResponse { hr_response };

        let encoded = ironrdp_core::encode_vec(&pdu).expect("CreateResponse encoding is infallible for valid inputs");
        self.outgoing.push_back(encoded);
    }
}

impl core::fmt::Debug for RdpemtTunnel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RdpemtTunnel")
            .field("side", &self.side)
            .field("state", &self.state)
            .field("outgoing_count", &self.outgoing.len())
            .field("events_count", &self.events.len())
            .finish()
    }
}
