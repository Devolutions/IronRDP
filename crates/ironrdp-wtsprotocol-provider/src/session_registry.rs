use std::collections::HashMap;

use parking_lot::Mutex;

#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub session_id: u32,
    pub connection_id: u32,
}

#[derive(Debug, Default)]
pub struct SessionRegistry {
    inner: Mutex<HashMap<u32, SessionEntry>>,
}

impl SessionRegistry {
    pub fn insert(&self, entry: SessionEntry) {
        self.inner.lock().insert(entry.connection_id, entry);
    }

    pub fn remove_by_connection_id(&self, connection_id: u32) -> Option<SessionEntry> {
        self.inner.lock().remove(&connection_id)
    }

    pub fn get_by_connection_id(&self, connection_id: u32) -> Option<SessionEntry> {
        self.inner.lock().get(&connection_id).cloned()
    }
}
