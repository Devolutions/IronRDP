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
        let connection = Arc::new(ProtocolConnection::new(connection_id, Arc::clone(&self.sessions)));

        self.active_connections.lock().push(Arc::clone(&connection));

        info!(connection_id, "Created protocol connection");

        connection
    }

    pub fn connection_count(&self) -> usize {
        self.active_connections.lock().len()
    }
}
