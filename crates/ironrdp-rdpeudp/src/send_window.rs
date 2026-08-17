//! Sender window buffer for tracking in-flight packets.
//!
//! MS-RDPEUDP2 Section 3.1.1.2.1.
//!
//! The send window tracks packets that have been transmitted but not yet
//! acknowledged. Each entry holds the packet data (for retransmission),
//! both sequence numbers (DataSeqNum and ChannelSeqNum), timing info,
//! and current state (Pending/Received/Lost).
//!
//! The window size is bounded by `1 << log_window_size` (max 32768).
//! The lower bound advances when consecutive entries at the front
//! transition to Received or Lost.
//!
//! When a packet is declared lost, its data is returned to the caller
//! for the ReliabilityController. The entry is drained from the window
//! (if at the front). Retransmission creates a fresh entry with a new
//! DataSeqNum but the same ChannelSeqNum, following the QUIC model.

use crate::time::MonotonicInstant;
use alloc::collections::VecDeque;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

/// State of a packet in the send window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendEntryState {
    /// Sent, waiting for acknowledgment.
    Pending,
    /// Acknowledged by the receiver.
    Received,
}

/// Metadata for a sent packet.
#[derive(Debug, Clone)]
pub(crate) struct SendEntry {
    /// DataSeqNum assigned when this packet was (re)transmitted.
    pub data_seq: u64,

    /// ChannelSeqNum: the logical sequence number for this data.
    /// Preserved across retransmissions so the receiver can match
    /// retransmits to the original gap.
    pub channel_seq: u64,

    /// When this packet was most recently sent.
    pub sent_at: MonotonicInstant,

    /// Payload size in bytes (used for congestion window accounting).
    pub size: usize,

    /// Current state: Pending or Received.
    pub state: SendEntryState,

    /// Number of times this packet has been transmitted.
    pub transmit_count: u32,
}

/// Information returned when a packet is declared lost.
#[derive(Debug, Clone)]
pub(crate) struct LostPacketInfo {
    /// ChannelSeqNum for matching with the retransmit queue.
    pub channel_seq: u64,
    /// The payload data for retransmission.
    pub data: Vec<u8>,
    /// Payload size in bytes (used for diagnostics and accounting in tests).
    #[cfg_attr(not(test), expect(dead_code, reason = "diagnostic field used in tests"))]
    pub size: usize,
}

/// Send window tracking all in-flight packets.
///
/// Internally a `VecDeque` where the front entry corresponds to the
/// lowest unresolved DataSeqNum. Entries at the front that have been
/// resolved (Received) are drained to advance the window.
#[derive(Debug)]
pub(crate) struct SendWindow {
    /// In-flight entries. Front = lowest DataSeqNum.
    entries: VecDeque<SendEntry>,

    /// Packet data indexed by DataSeqNum, held for retransmission.
    /// Removed when entry is Received or returned via mark_lost.
    data_store: VecDeque<(u64, Vec<u8>)>,

    /// Next DataSeqNum to assign to a new transmission.
    next_data_seq: u64,

    /// Next ChannelSeqNum to assign to new (non-retransmit) data.
    next_channel_seq: u64,

    /// Total bytes currently in flight (state == Pending).
    bytes_in_flight: u64,

    /// Maximum number of entries (1 << log_window_size).
    max_entries: usize,
}

impl SendWindow {
    /// Create a new send window.
    ///
    /// `initial_data_seq` and `initial_channel_seq` are the starting
    /// sequence numbers (typically derived from the handshake ISN).
    /// `log_window_size` determines the maximum window: `1 << log_window_size`.
    pub(crate) fn new(initial_data_seq: u64, initial_channel_seq: u64, log_window_size: u8) -> Self {
        let max_entries = 1usize << usize::from(log_window_size);
        Self {
            entries: VecDeque::with_capacity(max_entries),
            data_store: VecDeque::with_capacity(max_entries),
            next_data_seq: initial_data_seq,
            next_channel_seq: initial_channel_seq,
            bytes_in_flight: 0,
            max_entries,
        }
    }

    /// Whether the window has room for another packet.
    pub(crate) fn has_capacity(&self) -> bool {
        self.entries.len() < self.max_entries
    }

    /// Number of entries currently in the window.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the window is empty.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total bytes in flight (entries in Pending state).
    pub(crate) fn bytes_in_flight(&self) -> u64 {
        self.bytes_in_flight
    }

    /// The next DataSeqNum that will be assigned.
    pub(crate) fn next_data_seq(&self) -> u64 {
        self.next_data_seq
    }

    /// The next ChannelSeqNum that will be assigned to new data.
    #[cfg(test)]
    pub(crate) fn next_channel_seq(&self) -> u64 {
        self.next_channel_seq
    }

    /// Record a newly sent packet and return its assigned sequence numbers.
    ///
    /// The caller provides the payload data and the send timestamp.
    /// A new ChannelSeqNum is assigned (this is a first transmission,
    /// not a retransmit).
    ///
    /// Returns `None` if the window is full.
    pub(crate) fn push(&mut self, data: Vec<u8>, now: MonotonicInstant) -> Option<(u64, u64)> {
        if !self.has_capacity() {
            return None;
        }

        let data_seq = self.next_data_seq;
        let channel_seq = self.next_channel_seq;
        let size = data.len();

        self.entries.push_back(SendEntry {
            data_seq,
            channel_seq,
            sent_at: now,
            size,
            state: SendEntryState::Pending,
            transmit_count: 1,
        });
        self.data_store.push_back((data_seq, data));

        self.next_data_seq += 1;
        self.next_channel_seq += 1;
        self.bytes_in_flight += u64::try_from(size).expect("packet size fits in u64");

        Some((data_seq, channel_seq))
    }

    /// Record a retransmission with a preserved ChannelSeqNum.
    ///
    /// A new DataSeqNum is assigned (per RDPEUDP2: DataSeqNum changes
    /// on retransmit, ChannelSeqNum does not). A fresh Pending entry
    /// is added to the window.
    ///
    /// Returns `None` if the window is full.
    pub(crate) fn push_retransmit(&mut self, channel_seq: u64, data: Vec<u8>, now: MonotonicInstant) -> Option<u64> {
        if !self.has_capacity() {
            return None;
        }

        let data_seq = self.next_data_seq;
        let size = data.len();

        self.entries.push_back(SendEntry {
            data_seq,
            channel_seq,
            sent_at: now,
            size,
            state: SendEntryState::Pending,
            transmit_count: 2, // at least the second transmission
        });
        self.data_store.push_back((data_seq, data));

        self.next_data_seq += 1;
        self.bytes_in_flight += u64::try_from(size).expect("packet size fits in u64");

        Some(data_seq)
    }

    /// Mark a packet as received (acknowledged) by its DataSeqNum.
    ///
    /// Returns the size of the acknowledged packet (for congestion
    /// window accounting), or `None` if the seq was not found or
    /// was already received.
    pub(crate) fn mark_received(&mut self, data_seq: u64) -> Option<usize> {
        let entry = self.entries.iter_mut().find(|e| e.data_seq == data_seq)?;

        if entry.state != SendEntryState::Pending {
            return None;
        }

        entry.state = SendEntryState::Received;
        let size = entry.size;
        self.bytes_in_flight -= u64::try_from(size).expect("packet size fits in u64");

        // Remove data: no longer needed for retransmission.
        self.data_store.retain(|(seq, _)| *seq != data_seq);

        self.advance_window();

        Some(size)
    }

    /// Mark every pending packet up to and including `data_seq` as
    /// received, returning the total bytes that changed state.
    ///
    /// Walking the window rather than the sequence range keeps the cost
    /// tied to how many packets are in flight. Counting from the initial
    /// sequence number instead costs one iteration per sequence number
    /// ever sent, on every acknowledgment, and a peer naming a sequence
    /// number far ahead of the window pays for the whole gap.
    pub(crate) fn mark_received_through(&mut self, data_seq: u64) -> u64 {
        let mut bytes = 0;

        // Entries are ordered by DataSeqNum and `mark_received` drains the
        // resolved ones off the front, so this walks each acknowledged
        // packet once.
        while let Some(front) = self.entries.front() {
            if front.data_seq > data_seq {
                break;
            }

            let front_seq = front.data_seq;
            let Some(size) = self.mark_received(front_seq) else {
                break;
            };

            bytes += u64::try_from(size).expect("packet size fits in u64");
        }

        bytes
    }

    /// Mark a packet as lost by its DataSeqNum.
    ///
    /// Removes the entry from the window and returns its info
    /// (ChannelSeqNum + data) for the ReliabilityController to
    /// enqueue for retransmission.
    ///
    /// Returns `None` if not found or not Pending.
    pub(crate) fn mark_lost(&mut self, data_seq: u64) -> Option<LostPacketInfo> {
        let entry_idx = self.entries.iter().position(|e| e.data_seq == data_seq)?;

        if self.entries[entry_idx].state != SendEntryState::Pending {
            return None;
        }

        // Extract the entry.
        let entry = self.entries.remove(entry_idx).expect("index is valid");
        self.bytes_in_flight -= u64::try_from(entry.size).expect("packet size fits in u64");

        // Extract the data.
        let data = self
            .data_store
            .iter()
            .position(|(seq, _)| *seq == data_seq)
            .map(|i| self.data_store.remove(i).expect("index is valid").1)
            .unwrap_or_default();

        // Try to advance the window (front entries may now be resolved).
        self.advance_window();

        Some(LostPacketInfo {
            channel_seq: entry.channel_seq,
            data,
            size: entry.size,
        })
    }

    /// Get an entry by DataSeqNum (for loss detection timing checks).
    pub(crate) fn get_by_data_seq(&self, data_seq: u64) -> Option<&SendEntry> {
        self.entries.iter().find(|e| e.data_seq == data_seq)
    }

    /// Iterate over all Pending entries (for loss detection scans).
    pub(crate) fn pending_entries(&self) -> impl Iterator<Item = &SendEntry> {
        self.entries.iter().filter(|e| e.state == SendEntryState::Pending)
    }

    /// The lowest DataSeqNum still waiting to be acknowledged.
    ///
    /// This is what [MS-RDPEUDP2] 2.2.1.2.4 calls "the sequence number the
    /// Sender is waiting to receive acknowledgment of". Resolved entries are
    /// drained off the front, so the front is it; with nothing outstanding it
    /// is the next number that will be assigned.
    pub(crate) fn lowest_unacknowledged(&self) -> u64 {
        self.entries.front().map_or(self.next_data_seq, |entry| entry.data_seq)
    }

    /// The highest DataSeqNum that has been acknowledged.
    ///
    /// Returns `None` if no packets have been acknowledged yet.
    pub(crate) fn highest_acked_data_seq(&self) -> Option<u64> {
        self.entries
            .iter()
            .filter(|e| e.state == SendEntryState::Received)
            .map(|e| e.data_seq)
            .max()
    }

    /// Advance the window by draining front entries that are Received.
    fn advance_window(&mut self) {
        while let Some(front) = self.entries.front() {
            if front.state == SendEntryState::Pending {
                break;
            }
            let removed = self.entries.pop_front().expect("checked non-empty");
            // Clean up data store (should already be removed for Received entries,
            // but belt-and-suspenders).
            self.data_store.retain(|(seq, _)| *seq != removed.data_seq);
        }
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::*;

    fn now() -> MonotonicInstant {
        MonotonicInstant::from_millis(0)
    }

    fn later(base: MonotonicInstant, ms: u64) -> MonotonicInstant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn new_window_is_empty() {
        let w = SendWindow::new(1, 1, 6); // window = 64
        assert!(w.is_empty());
        assert_eq!(w.len(), 0);
        assert!(w.has_capacity());
        assert_eq!(w.bytes_in_flight(), 0);
        assert_eq!(w.next_data_seq(), 1);
        assert_eq!(w.next_channel_seq(), 1);
    }

    #[test]
    fn push_assigns_sequences() {
        let t = now();
        let mut w = SendWindow::new(10, 100, 6);

        let (dsn, csn) = w.push(vec![0xAA; 50], t).expect("has capacity");
        assert_eq!(dsn, 10);
        assert_eq!(csn, 100);
        assert_eq!(w.next_data_seq(), 11);
        assert_eq!(w.next_channel_seq(), 101);
        assert_eq!(w.len(), 1);
        assert_eq!(w.bytes_in_flight(), 50);
    }

    #[test]
    fn push_multiple() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);

        for i in 0..5 {
            let (dsn, csn) = w.push(vec![0; 100], t).expect("has capacity");
            assert_eq!(dsn, 1 + i);
            assert_eq!(csn, 1 + i);
        }

        assert_eq!(w.len(), 5);
        assert_eq!(w.bytes_in_flight(), 500);
    }

    #[test]
    fn push_rejects_when_full() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 2); // window = 4

        for _ in 0..4 {
            assert!(w.push(vec![0; 10], t).is_some());
        }

        assert!(!w.has_capacity());
        assert!(w.push(vec![0; 10], t).is_none());
    }

    #[test]
    fn mark_received_returns_size() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);
        w.push(vec![0; 200], t);

        let size = w.mark_received(1).expect("should find entry");
        assert_eq!(size, 200);
        assert_eq!(w.bytes_in_flight(), 0);
    }

    #[test]
    fn mark_received_unknown_seq_returns_none() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);
        w.push(vec![0; 100], t);

        assert!(w.mark_received(999).is_none());
    }

    #[test]
    fn mark_received_already_received_returns_none() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);
        w.push(vec![0; 100], t);

        w.mark_received(1);
        assert!(w.mark_received(1).is_none());
    }

    #[test]
    fn mark_lost_returns_info() {
        let t = now();
        let mut w = SendWindow::new(1, 100, 6);
        w.push(vec![0xBB; 50], t);

        let info = w.mark_lost(1).expect("should find entry");
        assert_eq!(info.channel_seq, 100);
        assert_eq!(info.data, vec![0xBB; 50]);
        assert_eq!(info.size, 50);
        assert_eq!(w.bytes_in_flight(), 0);
        // Entry was removed from the window.
        assert!(w.is_empty());
    }

    #[test]
    fn window_advances_on_front_resolved() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);

        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);

        assert_eq!(w.len(), 3);

        // ACK seq 1 → front drains
        w.mark_received(1);
        assert_eq!(w.len(), 2);

        // ACK seq 2 → front drains again
        w.mark_received(2);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn window_does_not_advance_past_pending() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);

        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);

        // ACK seq 3 (out of order): front is still seq 1 (Pending)
        w.mark_received(3);
        assert_eq!(w.len(), 3); // nothing drained

        // ACK seq 1 → front (seq 1, Received) drains.
        // Seq 2 (Pending) becomes the new front and blocks further drain.
        // Seq 3 (Received) remains behind seq 2.
        w.mark_received(1);
        assert_eq!(w.len(), 2); // seq 2 (Pending) and seq 3 (Received) remain
    }

    #[test]
    fn retransmit_via_push_retransmit() {
        let t = now();
        let t2 = later(t, 500);
        let mut w = SendWindow::new(1, 100, 6);

        w.push(vec![0xBB; 30], t);

        // Mark as lost: get the data back
        let info = w.mark_lost(1).expect("should find entry");
        assert_eq!(info.channel_seq, 100);
        assert_eq!(info.data, vec![0xBB; 30]);
        assert!(w.is_empty());

        // Retransmit with the same ChannelSeqNum but new DataSeqNum
        let new_dsn = w.push_retransmit(100, info.data, t2).expect("has capacity");
        assert_eq!(new_dsn, 2); // next_data_seq was 2
        assert_eq!(w.next_data_seq(), 3);
        assert_eq!(w.bytes_in_flight(), 30);
        assert_eq!(w.len(), 1);

        let entry = w.get_by_data_seq(2).expect("should find");
        assert_eq!(entry.channel_seq, 100);
        assert_eq!(entry.state, SendEntryState::Pending);
        assert_eq!(entry.transmit_count, 2);
        assert_eq!(entry.sent_at, t2);
    }

    #[test]
    fn pending_entries_iterator() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);

        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);

        w.mark_received(2);

        let pending: Vec<u64> = w.pending_entries().map(|e| e.data_seq).collect();
        assert_eq!(pending, vec![1, 3]);
    }

    #[test]
    fn highest_acked_data_seq() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);

        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);

        assert_eq!(w.highest_acked_data_seq(), None);

        w.mark_received(3);
        assert_eq!(w.highest_acked_data_seq(), Some(3));

        w.mark_received(1);
        assert_eq!(w.highest_acked_data_seq(), Some(3));
    }

    #[test]
    fn bytes_in_flight_tracking() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);

        w.push(vec![0; 100], t);
        w.push(vec![0; 200], t);
        w.push(vec![0; 300], t);
        assert_eq!(w.bytes_in_flight(), 600);

        w.mark_received(1);
        assert_eq!(w.bytes_in_flight(), 500);

        // Mark seq 2 lost: bytes leave in-flight
        let info = w.mark_lost(2).expect("should find");
        assert_eq!(w.bytes_in_flight(), 300);

        // Retransmit: bytes return to in-flight
        w.push_retransmit(info.channel_seq, info.data, t);
        assert_eq!(w.bytes_in_flight(), 500);
    }

    #[test]
    fn capacity_frees_after_advance() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 2); // window = 4

        for _ in 0..4 {
            w.push(vec![0; 10], t);
        }
        assert!(!w.has_capacity());

        // ACK front entry → window advances → capacity freed
        w.mark_received(1);
        assert!(w.has_capacity());
        assert!(w.push(vec![0; 10], t).is_some());
    }

    #[test]
    fn get_by_data_seq() {
        let t = now();
        let mut w = SendWindow::new(1, 100, 6);
        w.push(vec![0xAA; 10], t);

        let entry = w.get_by_data_seq(1).expect("should find");
        assert_eq!(entry.data_seq, 1);
        assert_eq!(entry.channel_seq, 100);

        assert!(w.get_by_data_seq(999).is_none());
    }

    #[test]
    fn mark_lost_removes_entry_from_window() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);

        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);

        // Mark middle packet as lost
        w.mark_lost(2);
        assert_eq!(w.len(), 2);

        // Remaining entries are seq 1 and seq 3
        let pending: Vec<u64> = w.pending_entries().map(|e| e.data_seq).collect();
        assert_eq!(pending, vec![1, 3]);
    }

    #[test]
    fn mark_lost_multiple() {
        let t = now();
        let mut w = SendWindow::new(1, 1, 6);

        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);
        w.push(vec![0; 10], t);

        let info1 = w.mark_lost(1).expect("should find");
        assert_eq!(info1.channel_seq, 1);

        let info3 = w.mark_lost(3).expect("should find");
        assert_eq!(info3.channel_seq, 3);

        // Only seq 2 remains
        assert_eq!(w.len(), 1);
        let entry = w.get_by_data_seq(2).expect("should find");
        assert_eq!(entry.channel_seq, 2);
    }

    #[test]
    fn retransmit_then_ack() {
        let t = now();
        let mut w = SendWindow::new(1, 100, 6);

        w.push(vec![0xAA; 50], t);
        w.push(vec![0xBB; 60], t);

        // Lose packet 1
        let info = w.mark_lost(1).expect("lost");
        assert_eq!(info.channel_seq, 100);

        // Retransmit it
        let new_dsn = w.push_retransmit(100, info.data, later(t, 500)).expect("has cap");
        assert_eq!(new_dsn, 3); // next_data_seq was 3

        // ACK both the original seq 2 and retransmitted seq 3
        w.mark_received(2);
        w.mark_received(3);

        assert!(w.is_empty());
        assert_eq!(w.bytes_in_flight(), 0);
    }
}
