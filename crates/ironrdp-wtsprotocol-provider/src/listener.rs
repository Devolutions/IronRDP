use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tracing::info;

use crate::connection::ProtocolConnection;
use crate::session_registry::SessionRegistry;

#[derive(Debug)]
pub struct ProtocolListener {
    next_connection_id: AtomicU32,
    sessions: Arc<SessionRegistry>,
    active_connections: Mutex<Vec<Arc<ProtocolConnection>>>,
}

impl ProtocolListener {
    pub fn new(sessions: Arc<SessionRegistry>) -> Self {
        Self {
            next_connection_id: AtomicU32::new(1),
            sessions,
            active_connections: Mutex::new(Vec::new()),
        }
    }

    pub fn create_connection(&self) -> Arc<ProtocolConnection> {
        let connection_id = self.next_connection_id.fetch_add(1, Ordering::Relaxed);
        self.create_connection_with_id(connection_id)
    }

    pub fn create_connection_with_id(&self, connection_id: u32) -> Arc<ProtocolConnection> {
        self.advance_next_connection_id(connection_id);
        let connection = Arc::new(ProtocolConnection::new(connection_id, Arc::clone(&self.sessions)));

        self.active_connections.lock().push(Arc::clone(&connection));

        info!(connection_id, "Created protocol connection");

        connection
    }

    fn advance_next_connection_id(&self, connection_id: u32) {
        let next_candidate = connection_id.saturating_add(1);

        loop {
            let current = self.next_connection_id.load(Ordering::Relaxed);

            if current >= next_candidate {
                return;
            }

            if self
                .next_connection_id
                .compare_exchange(current, next_candidate, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
        }
    }

    pub fn connection_count(&self) -> usize {
        self.active_connections.lock().len()
    }
}
