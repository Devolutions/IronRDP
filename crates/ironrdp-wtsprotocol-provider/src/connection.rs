use std::sync::Arc;

use parking_lot::Mutex;
use tracing::{debug, info, warn};

use crate::session_registry::{SessionEntry, SessionRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionLifecycleState {
    Created,
    Accepted,
    SessionNotified,
    Connected,
    LoggedOn,
    Disconnected,
    Closed,
}

#[derive(Debug)]
pub struct ProtocolConnection {
    connection_id: u32,
    state: Mutex<ConnectionLifecycleState>,
    sessions: Arc<SessionRegistry>,
}

impl ProtocolConnection {
    pub fn new(connection_id: u32, sessions: Arc<SessionRegistry>) -> Self {
        Self {
            connection_id,
            state: Mutex::new(ConnectionLifecycleState::Created),
            sessions,
        }
    }

    pub fn connection_id(&self) -> u32 {
        self.connection_id
    }

    pub fn accept_connection(&self) -> Result<(), &'static str> {
        let mut state = self.state.lock();
        if *state != ConnectionLifecycleState::Created {
            return Err(self.invalid_transition("AcceptConnection", *state));
        }

        *state = ConnectionLifecycleState::Accepted;
        info!(connection_id = self.connection_id, "Accepted protocol connection");

        Ok(())
    }

    pub fn notify_session_id(&self, session_id: u32) -> Result<(), &'static str> {
        let mut state = self.state.lock();
        if !matches!(
            *state,
            ConnectionLifecycleState::Accepted | ConnectionLifecycleState::SessionNotified
        ) {
            return Err(self.invalid_transition("NotifySessionId", *state));
        }

        self.sessions.insert(SessionEntry {
            session_id,
            connection_id: self.connection_id,
        });

        *state = ConnectionLifecycleState::SessionNotified;
        info!(connection_id = self.connection_id, session_id, "Notified session id");

        Ok(())
    }

    pub fn connect_notify(&self) -> Result<(), &'static str> {
        let mut state = self.state.lock();
        if !matches!(
            *state,
            ConnectionLifecycleState::SessionNotified | ConnectionLifecycleState::Connected
        ) {
            return Err(self.invalid_transition("ConnectNotify", *state));
        }

        *state = ConnectionLifecycleState::Connected;
        debug!(connection_id = self.connection_id, "Connection entered connected state");

        Ok(())
    }

    pub fn logon_notify(&self) -> Result<(), &'static str> {
        let mut state = self.state.lock();
        if !matches!(
            *state,
            ConnectionLifecycleState::Connected | ConnectionLifecycleState::LoggedOn
        ) {
            return Err(self.invalid_transition("LogonNotify", *state));
        }

        *state = ConnectionLifecycleState::LoggedOn;
        info!(connection_id = self.connection_id, "Logon notified");

        Ok(())
    }

    pub fn disconnect_notify(&self) -> Result<(), &'static str> {
        let mut state = self.state.lock();
        if matches!(
            *state,
            ConnectionLifecycleState::Created | ConnectionLifecycleState::Accepted
        ) {
            return Err(self.invalid_transition("DisconnectNotify", *state));
        }

        *state = ConnectionLifecycleState::Disconnected;
        info!(connection_id = self.connection_id, "Disconnect notified");

        Ok(())
    }

    pub fn close(&self) -> Result<(), &'static str> {
        let mut state = self.state.lock();
        if *state == ConnectionLifecycleState::Closed {
            return Err(self.invalid_transition("Close", *state));
        }

        self.sessions.remove_by_connection_id(self.connection_id);

        *state = ConnectionLifecycleState::Closed;
        info!(connection_id = self.connection_id, "Closed protocol connection");

        Ok(())
    }

    pub fn state(&self) -> ConnectionLifecycleState {
        *self.state.lock()
    }

    pub fn session_id(&self) -> Option<u32> {
        self.sessions
            .get_by_connection_id(self.connection_id)
            .map(|entry| entry.session_id)
    }

    fn invalid_transition(&self, method_name: &'static str, current_state: ConnectionLifecycleState) -> &'static str {
        warn!(
            connection_id = self.connection_id,
            ?current_state,
            method = method_name,
            "Invalid connection lifecycle transition"
        );

        "invalid connection lifecycle transition"
    }
}
