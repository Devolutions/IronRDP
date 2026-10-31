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
    /// Id of the last bridge made (one per connection).
    last_bridge: u64,
    /// Id of the bridge whose channel the client opened, if any.
    open_bridge: Option<u64>,
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
    /// channel open. False before the client answered the DVC Create Request,
    /// when it declined the channel, and after the connection ended. The server
    /// drops a request sent while this is false.
    pub fn is_channel_open(&self) -> bool {
        self.lock_state().open_bridge.is_some()
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

    fn new_bridge_id(&self) -> u64 {
        let mut state = self.lock_state();
        state.last_bridge += 1;
        state.last_bridge
    }

    fn set_open(&self, bridge: u64, open: bool) {
        let mut state = self.lock_state();
        if open {
            state.open_bridge = Some(bridge);
        } else if state.open_bridge == Some(bridge) {
            // A newer connection may have opened its channel already.
            state.open_bridge = None;
        }
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
    /// Tells this connection's channel state apart from that of another one.
    id: u64,
}

impl EchoDvcBridge {
    pub fn new(handle: EchoServerHandle) -> Self {
        let id = handle.new_bridge_id();
        Self {
            inner: EchoServer::new(),
            handle,
            id,
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
        self.handle.set_open(self.id, true);
        self.inner.start(channel_id)
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let messages = self.inner.process(channel_id, payload)?;
        self.handle.on_response(payload);
        Ok(messages)
    }

    fn close(&mut self, channel_id: u32) {
        self.handle.set_open(self.id, false);
        self.inner.close(channel_id)
    }
}

impl DvcServerProcessor for EchoDvcBridge {}

pub(crate) fn build_echo_request(payload: Vec<u8>) -> ServerResult<DvcMessage> {
    EchoServer::request_message(payload).map_err_kind("build ECHO request message", ServerErrorKind::Pdu)
}

#[cfg(test)]
mod tests {
    use ironrdp_dvc::DrdynvcServer;
    use ironrdp_dvc::pdu::{
        CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateResponsePdu, CreationStatus, DrdynvcClientPdu,
    };
    use ironrdp_svc::SvcProcessor as _;

    use super::*;

    fn handle() -> EchoServerHandle {
        let (sender, _receiver) = mpsc::unbounded_channel();
        EchoServerHandle::new(sender)
    }

    fn client_sends(server: &mut DrdynvcServer, pdu: DrdynvcClientPdu) {
        server.process(&ironrdp_core::encode_vec(&pdu).unwrap()).unwrap();
    }

    /// A DVC server with the ECHO bridge, after the client sent its capabilities, and the ECHO channel id.
    fn server(handle: &EchoServerHandle) -> (DrdynvcServer, u32) {
        let mut server = DrdynvcServer::new().with_dynamic_channel(EchoDvcBridge::new(handle.clone()));
        client_sends(
            &mut server,
            DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V2)),
        );
        let id = server.get_channel_id_by_type::<EchoDvcBridge>().unwrap();
        (server, id)
    }

    fn client_answers(server: &mut DrdynvcServer, id: u32, status: CreationStatus) {
        client_sends(server, DrdynvcClientPdu::Create(CreateResponsePdu::new(id, status)));
    }

    // oracle: regression #2339
    #[test]
    fn open_after_the_client_accepts_until_it_closes() {
        let handle = handle();
        let (mut server, id) = server(&handle);
        assert!(!handle.is_channel_open(), "Create Request sent, no answer yet");
        client_answers(&mut server, id, CreationStatus::OK);
        assert!(handle.is_channel_open());
        client_sends(&mut server, DrdynvcClientPdu::Close(ClosePdu::new(id)));
        assert!(!handle.is_channel_open(), "the client closed the channel");
    }

    // oracle: regression #2339
    #[test]
    fn not_open_when_the_client_declines() {
        let handle = handle();
        let (mut server, id) = server(&handle);
        client_answers(&mut server, id, CreationStatus::NO_LISTENER);
        assert!(!handle.is_channel_open());
    }

    // oracle: regression #2339
    #[test]
    fn not_open_after_the_connection_ends() {
        let handle = handle();
        let (mut server, id) = server(&handle);
        client_answers(&mut server, id, CreationStatus::OK);
        drop(server);
        assert!(!handle.is_channel_open());
    }

    // oracle: mutant the end of an old connection clearing the open channel of the next one
    #[test]
    fn the_end_of_an_old_connection_keeps_the_new_channel_open() {
        let handle = handle();
        let (mut old, old_id) = server(&handle);
        client_answers(&mut old, old_id, CreationStatus::OK);
        let (mut new, new_id) = server(&handle);
        client_answers(&mut new, new_id, CreationStatus::OK);
        drop(old);
        assert!(handle.is_channel_open(), "the new connection's channel stays open");
        drop(new);
        assert!(!handle.is_channel_open());
    }
}
