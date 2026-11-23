use std::collections::VecDeque;

use crate::{
    PduSource,
    process::{ReplayError, ReplayErrorKind, ReplayResult},
};

/// A single timestamped PDU stored in [`PduBuffer`].
pub(crate) struct PduEntry {
    pub(crate) timestamp_ms: f64,
    pub(crate) source: PduSource,
    pub(crate) data: Box<[u8]>,
}

pub struct PduBuffer {
    entries: VecDeque<PduEntry>,
}

impl Default for PduBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl PduBuffer {
    pub fn new() -> Self {
        PduBuffer {
            entries: VecDeque::new(),
        }
    }

    /// Appends a PDU to the back of the buffer.
    ///
    /// Returns an error and leaves the buffer unchanged when `timestamp_ms` is not finite.
    pub fn push_pdu(&mut self, timestamp_ms: f64, source: PduSource, data: &[u8]) -> ReplayResult<()> {
        if timestamp_ms.is_finite() {
            self.entries.push_back(PduEntry {
                timestamp_ms,
                source,
                data: data.into(),
            });
            Ok(())
        } else {
            Err(ReplayError::new("replay", ReplayErrorKind::InvalidTimestamp))
        }
    }

    /// Returns the timestamp of the next (front) PDU, or `None` if empty.
    pub fn peek_timestamp(&self) -> Option<f64> {
        self.entries.front().map(|e| e.timestamp_ms)
    }

    /// Removes and returns the front PDU, or `None` if empty.
    pub(crate) fn pop_pdu(&mut self) -> Option<PduEntry> {
        self.entries.pop_front()
    }

    /// Returns the source direction of the next PDU without consuming it.
    pub fn peek_source(&self) -> Option<PduSource> {
        self.entries.front().map(|e| e.source)
    }

    /// Returns the number of PDUs currently in the buffer.
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// Discards all buffered PDUs.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
