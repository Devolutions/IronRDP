use std::sync::Arc;

use tracing::info;

use crate::listener::ProtocolListener;
use crate::session_registry::SessionRegistry;

#[derive(Debug)]
pub struct ProtocolManager {
    sessions: Arc<SessionRegistry>,
}

impl Default for ProtocolManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(SessionRegistry::default()),
        }
    }

    pub fn create_listener(&self) -> ProtocolListener {
        info!("Creating protocol listener");
        ProtocolListener::new(Arc::clone(&self.sessions))
    }

    pub fn sessions(&self) -> Arc<SessionRegistry> {
        Arc::clone(&self.sessions)
    }
}
