use std::collections::VecDeque;
use wasm_bindgen::prelude::*;

use crate::PduSource;

/// A single timestamped PDU stored in [`PduBuffer`].
pub(crate) struct PduEntry {
    pub(crate) timestamp_ms: f64,
    pub(crate) source: PduSource,
    pub(crate) data: Box<[u8]>,
}

#[wasm_bindgen]
pub struct PduBuffer {
    entries: VecDeque<PduEntry>,
}

impl Default for PduBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl PduBuffer {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        PduBuffer {
            entries: VecDeque::new(),
        }
    }

    /// Append a PDU to the back of the buffer.
    #[wasm_bindgen]
    pub fn push_pdu(&mut self, timestamp_ms: f64, source: PduSource, data: &[u8]) {
        self.entries.push_back(PduEntry {
            timestamp_ms,
            source,
            data: data.into(),
        });
    }

    /// Returns the timestamp of the next (front) PDU, or `NAN` if empty.
    ///
    /// `NAN` comparisons are always false, so JS callers using `<= target_ms`
    /// naturally stop when the buffer is empty.
    #[wasm_bindgen(js_name = peekTimestamp)]
    pub fn peek_timestamp_js(&self) -> f64 {
        self.peek_timestamp().unwrap_or(f64::NAN)
    }

    /// Returns the timestamp of the last (back) PDU, or `NAN` if empty.
    #[wasm_bindgen(js_name = peekLastTimestamp)]
    pub fn peek_last_timestamp_js(&self) -> f64 {
        self.peek_last_timestamp().unwrap_or(f64::NAN)
    }

    /// Returns the timestamp of the next (front) PDU, or `None` if empty.
    pub fn peek_timestamp(&self) -> Option<f64> {
        self.entries.front().map(|e| e.timestamp_ms)
    }

    /// Returns the timestamp of the last (back) PDU, or `None` if empty.
    pub fn peek_last_timestamp(&self) -> Option<f64> {
        self.entries.back().map(|e| e.timestamp_ms)
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
