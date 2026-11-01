use core::time::Duration;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_echo::server::EchoServer;
use ironrdp_error::ResultExt as _;
use ironrdp_pdu::PduResult;
use tokio::sync::mpsc;

use crate::error::{ServerError, ServerErrorExt as _, ServerErrorKind, ServerResult};
use crate::server::ServerEvent;

#[derive(Debug, Clone)]
pub struct EchoRoundTripMeasurement {
    pub payload: Vec<u8>,
    pub round_trip_time: Duration,
}

#[derive(Debug)]
pub enum EchoServerMessage {
    SendRequest { payload: Vec<u8> },
}

impl core::fmt::Display for EchoServerMessage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SendRequest { payload } => write!(f, "SendRequest(size={})", payload.len()),
        }
    }
}

#[derive(Debug, Default)]
struct EchoHandleState {
    pending: BTreeMap<Vec<u8>, VecDeque<Instant>>,
    measurements: VecDeque<EchoRoundTripMeasurement>,
    /// The client of the current connection has the ECHO channel open.
    channel_open: bool,
}

/// Shared handle for runtime ECHO requests and RTT measurements.
#[derive(Debug, Clone)]
pub struct EchoServerHandle {
    sender: mpsc::UnboundedSender<ServerEvent>,
    state: Arc<Mutex<EchoHandleState>>,
}

impl EchoServerHandle {
    pub(crate) fn new(sender: mpsc::UnboundedSender<ServerEvent>) -> Self {
        Self {
            sender,
            state: Arc::new(Mutex::new(EchoHandleState::default())),
        }
    }

    /// Sends a runtime ECHO request.
    ///
    /// The payload must be at least one byte, as required by MS-RDPEECO section 3.1.5.1.
    pub fn send_request(&self, payload: Vec<u8>) -> ServerResult<()> {
        if payload.is_empty() {
            return Err(ServerError::reason("echo request", "payload must be at least one byte"));
        }

        self.sender
            .send(ServerEvent::Echo(EchoServerMessage::SendRequest { payload }))
            .map_err(|_error| ServerError::channel("send ECHO request event"))
    }

    /// True while the client of the current connection has the ECHO dynamic
    /// channel open: from its [2.2.2.2] DVC Create Response PDU (DYNVC_CREATE_RSP)
    /// with status OK until [2.2.4] Closing a DVC (DYNVC_CLOSE) or the end of the
    /// connection. False before the client answers, and when its response has
    /// another status. The server drops a request sent while this is false.
    ///
    /// The handle tracks one connection at a time, as `RdpServer` serves them.
    ///
    /// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/8f284ea3-54f3-4c24-8168-8a001c63b581
    /// [2.2.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/c02dfd21-ccbc-4254-985b-3ef6dd115dec
    pub fn is_channel_open(&self) -> bool {
        self.lock_state().channel_open
    }

    /// Drains collected RTT measurements.
    pub fn take_measurements(&self) -> Vec<EchoRoundTripMeasurement> {
        let mut state = self.lock_state();
        state.measurements.drain(..).collect()
    }

    pub(crate) fn on_request_sent(&self, payload: &[u8]) {
        let mut state = self.lock_state();
        state
            .pending
            .entry(payload.to_vec())
            .or_default()
            .push_back(Instant::now());
    }

    fn on_response(&self, payload: &[u8]) {
        let mut state = self.lock_state();
        let Some(sent_at_queue) = state.pending.get_mut(payload) else {
            return;
        };

        let Some(sent_at) = sent_at_queue.pop_front() else {
            return;
        };

        if sent_at_queue.is_empty() {
            state.pending.remove(payload);
        }

        state.measurements.push_back(EchoRoundTripMeasurement {
            payload: payload.to_vec(),
            round_trip_time: sent_at.elapsed(),
        });
    }

    fn on_channel_opened(&self) {
        self.lock_state().channel_open = true;
    }

    fn on_channel_closed(&self) {
        self.lock_state().channel_open = false;
    }

    fn lock_state(&self) -> MutexGuard<'_, EchoHandleState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// DVC bridge for ECHO that tracks RTT on responses.
pub struct EchoDvcBridge {
    inner: EchoServer,
    handle: EchoServerHandle,
}

impl EchoDvcBridge {
    pub fn new(handle: EchoServerHandle) -> Self {
        Self {
            inner: EchoServer::new(),
            handle,
        }
    }

    pub fn handle(&self) -> &EchoServerHandle {
        &self.handle
    }
}

impl_as_any!(EchoDvcBridge);

impl DvcProcessor for EchoDvcBridge {
    fn channel_name(&self) -> &str {
        self.inner.channel_name()
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.handle.on_channel_opened();
        self.inner.start(channel_id)
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let messages = self.inner.process(channel_id, payload)?;
        self.handle.on_response(payload);
        Ok(messages)
    }

    fn close(&mut self, channel_id: u32) {
        self.handle.on_channel_closed();
        self.inner.close(channel_id)
    }
}

impl DvcServerProcessor for EchoDvcBridge {}

pub(crate) fn build_echo_request(payload: Vec<u8>) -> ServerResult<DvcMessage> {
    EchoServer::request_message(payload).map_err_kind("build ECHO request message", ServerErrorKind::Pdu)
}
