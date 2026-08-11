//! Reliability controller for retransmission management.
//!
//! MS-RDPEUDP2 Section 3.1.1.2.4.
//!
//! Manages the retransmit queue: when packets are declared lost,
//! their ChannelSeqNum is enqueued for retransmission. The retransmit
//! queue is drained by `poll_transmit()` in the connection state machine.
//!
//! Key invariant: ChannelSeqNum is preserved across retransmissions
//! so the receiver can match a retransmit to the original gap.
//! DataSeqNum changes on each transmission (assigned by SendWindow).

use alloc::collections::VecDeque;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

/// Entry in the retransmit queue.
#[derive(Debug, Clone)]
pub(crate) struct RetransmitEntry {
    /// The ChannelSeqNum identifying this data across retransmissions.
    pub channel_seq: u64,

    /// The payload data to retransmit.
    pub data: Vec<u8>,
}

/// Retransmission queue manager.
///
/// When the loss detector declares packets lost, their data is enqueued
/// here by ChannelSeqNum. The connection state machine dequeues entries
/// when it has congestion window budget to retransmit them.
#[derive(Debug)]
pub(crate) struct ReliabilityController {
    /// Queue of packets awaiting retransmission.
    queue: VecDeque<RetransmitEntry>,
}

impl ReliabilityController {
    /// Create an empty controller.
    pub(crate) fn new() -> Self {
        Self { queue: VecDeque::new() }
    }

    /// Enqueue data for retransmission.
    ///
    /// If the same `channel_seq` is already queued, the existing entry
    /// is replaced (avoids duplicate retransmits).
    pub(crate) fn enqueue(&mut self, channel_seq: u64, data: Vec<u8>) {
        // Avoid duplicate entries for the same channel_seq.
        if self.queue.iter().any(|e| e.channel_seq == channel_seq) {
            return;
        }
        self.queue.push_back(RetransmitEntry { channel_seq, data });
    }

    /// Dequeue the next entry for retransmission.
    ///
    /// Returns `None` if the queue is empty.
    pub(crate) fn dequeue(&mut self) -> Option<RetransmitEntry> {
        self.queue.pop_front()
    }

    /// Peek at the next entry without removing it.
    #[cfg(test)]
    pub(crate) fn peek(&self) -> Option<&RetransmitEntry> {
        self.queue.front()
    }

    /// Whether the queue has entries.
    pub(crate) fn has_pending(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Number of entries in the queue.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    /// Remove an entry by channel_seq if it's no longer needed
    /// (e.g., the receiver has since acknowledged it).
    #[cfg(test)]
    pub(crate) fn cancel(&mut self, channel_seq: u64) {
        self.queue.retain(|e| e.channel_seq != channel_seq);
    }

    /// Clear the entire queue.
    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.queue.clear();
    }
}

impl Default for ReliabilityController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let rc = ReliabilityController::new();
        assert!(!rc.has_pending());
        assert_eq!(rc.len(), 0);
    }

    #[test]
    fn enqueue_and_dequeue() {
        let mut rc = ReliabilityController::new();

        rc.enqueue(100, vec![0xAA]);
        rc.enqueue(101, vec![0xBB]);

        assert!(rc.has_pending());
        assert_eq!(rc.len(), 2);

        let entry = rc.dequeue().expect("should have entry");
        assert_eq!(entry.channel_seq, 100);
        assert_eq!(entry.data, vec![0xAA]);

        let entry = rc.dequeue().expect("should have entry");
        assert_eq!(entry.channel_seq, 101);
        assert_eq!(entry.data, vec![0xBB]);

        assert!(!rc.has_pending());
    }

    #[test]
    fn dequeue_fifo_order() {
        let mut rc = ReliabilityController::new();

        for i in 0..5 {
            rc.enqueue(i, vec![u8::try_from(i).expect("fits")]);
        }

        for i in 0..5 {
            let entry = rc.dequeue().expect("should have entry");
            assert_eq!(entry.channel_seq, i);
        }
    }

    #[test]
    fn duplicate_enqueue_ignored() {
        let mut rc = ReliabilityController::new();

        rc.enqueue(100, vec![0xAA]);
        rc.enqueue(100, vec![0xBB]); // duplicate, ignored

        assert_eq!(rc.len(), 1);

        let entry = rc.dequeue().expect("should have entry");
        assert_eq!(entry.data, vec![0xAA]); // original data preserved
    }

    #[test]
    fn peek() {
        let mut rc = ReliabilityController::new();

        assert!(rc.peek().is_none());

        rc.enqueue(100, vec![0xAA]);
        let peeked = rc.peek().expect("should have entry");
        assert_eq!(peeked.channel_seq, 100);

        // Peek doesn't remove
        assert_eq!(rc.len(), 1);
    }

    #[test]
    fn cancel() {
        let mut rc = ReliabilityController::new();

        rc.enqueue(100, vec![0xAA]);
        rc.enqueue(101, vec![0xBB]);
        rc.enqueue(102, vec![0xCC]);

        rc.cancel(101);

        assert_eq!(rc.len(), 2);
        let entry = rc.dequeue().expect("entry");
        assert_eq!(entry.channel_seq, 100);
        let entry = rc.dequeue().expect("entry");
        assert_eq!(entry.channel_seq, 102);
    }

    #[test]
    fn cancel_nonexistent() {
        let mut rc = ReliabilityController::new();
        rc.enqueue(100, vec![0xAA]);

        rc.cancel(999); // no-op
        assert_eq!(rc.len(), 1);
    }

    #[test]
    fn clear() {
        let mut rc = ReliabilityController::new();

        rc.enqueue(100, vec![0xAA]);
        rc.enqueue(101, vec![0xBB]);

        rc.clear();
        assert!(!rc.has_pending());
        assert_eq!(rc.len(), 0);
    }

    #[test]
    fn default_matches_new() {
        let from_new = ReliabilityController::new();
        let from_default = ReliabilityController::default();
        assert_eq!(from_new.len(), from_default.len());
    }
}
