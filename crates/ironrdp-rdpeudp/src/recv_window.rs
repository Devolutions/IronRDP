//! Receiver window buffer for packet reordering and reassembly.
//!
//! MS-RDPEUDP2 Section 3.1.1.2.2.
//!
//! The receive window tracks expected and received packets, handles
//! out-of-order delivery, and reassembles data in ChannelSeqNum order
//! for delivery to the application layer.
//!
//! The window tracks packet state using DataSeqNum for ACK generation,
//! and uses ChannelSeqNum for data reassembly ordering.

use alloc::collections::BTreeMap;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

/// State of a slot in the receive window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecvEntryState {
    /// Expected but not yet received.
    Pending,
    /// Packet received.
    Received,
}

/// Entry in the receive window, tracking a single DataSeqNum slot.
#[derive(Debug, Clone)]
struct RecvEntry {
    state: RecvEntryState,
    /// ChannelSeqNum associated with this data, if received.
    channel_seq: Option<u64>,
}

/// Receive window tracking incoming packets.
///
/// Uses DataSeqNum to track which packets have been received (for ACK
/// vector generation), and ChannelSeqNum for reassembly ordering.
///
/// Data is delivered to the application in ChannelSeqNum order:
/// contiguous data starting from `next_channel_seq` is drained
/// from the reorder buffer.
#[derive(Debug)]
pub(crate) struct RecvWindow {
    /// Per-slot state indexed by (data_seq - base_seq).
    entries: Vec<RecvEntry>,

    /// DataSeqNum of the first slot in `entries`.
    base_seq: u64,

    /// Highest DataSeqNum received so far.
    highest_seq: u64,

    /// Next ChannelSeqNum expected for in-order delivery.
    next_channel_seq: u64,

    /// Out-of-order data buffered by ChannelSeqNum, waiting for gaps to fill.
    reorder_buf: BTreeMap<u64, Vec<u8>>,

    /// Maximum window size (1 << log_window_size).
    max_entries: usize,
}

impl RecvWindow {
    /// Create a new receive window.
    ///
    /// `initial_data_seq` is the first DataSeqNum expected.
    /// `initial_channel_seq` is the first ChannelSeqNum expected.
    /// `log_window_size` determines the maximum window.
    pub(crate) fn new(initial_data_seq: u64, initial_channel_seq: u64, log_window_size: u8) -> Self {
        let max_entries = 1usize << usize::from(log_window_size);
        Self {
            entries: Vec::with_capacity(max_entries),
            base_seq: initial_data_seq,
            highest_seq: initial_data_seq.saturating_sub(1),
            next_channel_seq: initial_channel_seq,
            reorder_buf: BTreeMap::new(),
            max_entries,
        }
    }

    /// Process a received data packet.
    ///
    /// Records the DataSeqNum as received and buffers the data for
    /// ChannelSeqNum-ordered reassembly. Returns `true` if this is a
    /// new packet (not a duplicate).
    pub(crate) fn receive(&mut self, data_seq: u64, channel_seq: u64, data: Vec<u8>) -> bool {
        // A ChannelSeqNum below the delivery cursor belongs to data the
        // application has already seen, so this is a retransmission of
        // something we no longer need. Acknowledge it, so the sender stops
        // resending, but keep it out of the reorder buffer: `drain_ordered`
        // only ever removes at `next_channel_seq`, so an entry below that
        // would occupy a slot for the rest of the connection.
        let already_delivered = channel_seq < self.next_channel_seq;

        // The reorder buffer holds at most one entry per window slot. It
        // drains only at `next_channel_seq`, so a peer that keeps sending
        // fresh numbers while withholding that one would otherwise grow it by
        // an entry per datagram, without bound. Refuse such a packet outright
        // rather than storing it and acknowledging it.
        //
        // Never refuse the packet at `next_channel_seq` itself. It is the one
        // that empties the buffer, and it is by definition not in it, so a
        // plain "full, and not already present" test rejects precisely the
        // packet that would resolve the situation and reassembly stops for
        // good.
        if !already_delivered
            && channel_seq != self.next_channel_seq
            && self.reorder_buf.len() >= self.max_entries
            && !self.reorder_buf.contains_key(&channel_seq)
        {
            return false;
        }

        if !self.occupy_slot(data_seq, Some(channel_seq)) {
            return false;
        }

        // Buffer data for reassembly by ChannelSeqNum.
        if !already_delivered {
            self.reorder_buf.insert(channel_seq, data);
        }

        true
    }

    /// Record a DataSeqNum as received while taking nothing from the packet.
    ///
    /// This is a dummy packet. [MS-RDPEUDP2] 3.1.1.1.5 has the transport treat
    /// one "as a normal RDP-UDP2 packet", so it occupies its sequence number
    /// and is acknowledged like any other, but "the contents MUST be ignored
    /// by higher layers", which includes the ChannelSeqNum it carries: that
    /// number belongs to no real datum and reassembling around it would leave
    /// the delivery cursor waiting on data no one will ever send.
    pub(crate) fn receive_without_payload(&mut self, data_seq: u64) -> bool {
        self.occupy_slot(data_seq, None)
    }

    /// Mark a slot received, returning whether this is new rather than a
    /// duplicate or a packet outside the window.
    fn occupy_slot(&mut self, data_seq: u64, channel_seq: Option<u64>) -> bool {
        // Ignore packets below the window base (already processed).
        if data_seq < self.base_seq {
            return false;
        }

        let index = data_seq - self.base_seq;

        // Ignore packets beyond the window limit.
        if index >= u64::try_from(self.max_entries).expect("max_entries fits in u64") {
            return false;
        }

        let idx = usize::try_from(index).expect("index within window fits in usize");

        // Extend the entries vector if needed.
        while self.entries.len() <= idx {
            self.entries.push(RecvEntry {
                state: RecvEntryState::Pending,
                channel_seq: None,
            });
        }

        // Check for duplicate.
        if self.entries[idx].state == RecvEntryState::Received {
            return false;
        }

        // Mark as received.
        self.entries[idx].state = RecvEntryState::Received;
        self.entries[idx].channel_seq = channel_seq;

        if data_seq > self.highest_seq {
            self.highest_seq = data_seq;
        }

        true
    }

    /// Drain contiguous in-order data starting from `next_channel_seq`.
    ///
    /// Returns data chunks in ChannelSeqNum order. The caller should
    /// call this after each `receive()` to deliver data to the
    /// application layer.
    pub(crate) fn drain_ordered(&mut self) -> Vec<Vec<u8>> {
        let mut delivered = Vec::new();

        while let Some(data) = self.reorder_buf.remove(&self.next_channel_seq) {
            delivered.push(data);
            self.next_channel_seq += 1;
        }

        delivered
    }

    /// Advance the lower bound past the received packets at the bottom of
    /// the window, which is what sending an acknowledgment that names them
    /// permits.
    ///
    /// [MS-RDPEUDP2] 3.1.1.2.2 gives two events that move the lower bound:
    /// this one and an AckOfAcks from the sender (`advance_base`). Only
    /// implementing the second stalls a connection that is not losing
    /// anything, because a sender with nothing to report sends no ACK
    /// vector, so no AckOfAcks comes back, so the bound never moves and
    /// `receive` starts rejecting everything one window past the last
    /// AckOfAcks.
    ///
    /// The spec's caveat for the vector case ("the lower bound can advance
    /// only if the first sequence number ... is in the Received state")
    /// falls out of taking the leading run: a hole at the bottom is a run
    /// of length zero.
    ///
    /// Returns the number of slots released.
    pub(crate) fn release_acknowledged(&mut self) -> usize {
        let received = self
            .entries
            .iter()
            .take_while(|entry| entry.state == RecvEntryState::Received)
            .count();

        if received > 0 {
            self.entries.drain(..received);
            self.base_seq += u64::try_from(received).expect("run length fits in u64");
        }

        received
    }

    /// Advance the window base up to `new_base`.
    ///
    /// Called when an AckOfAcks is received, telling us the sender
    /// knows we've received everything up to this point. Entries below
    /// `new_base` are no longer needed for ACK vector generation.
    pub(crate) fn advance_base(&mut self, new_base: u64) {
        if new_base <= self.base_seq {
            return;
        }

        let advance_by = new_base - self.base_seq;
        let drain_count =
            usize::try_from(advance_by.min(u64::try_from(self.entries.len()).expect("entries len fits in u64")))
                .expect("drain count fits in usize");

        self.entries.drain(..drain_count);
        self.base_seq = new_base;
    }

    /// Build an ACK vector representing the reception state of packets
    /// from `base_seq` to `highest_seq`.
    ///
    /// Returns a list of `(state, count)` pairs suitable for encoding
    /// into the RDPEUDP2 AckVector format. The receiver generates this
    /// to tell the sender which packets it has and hasn't received.
    ///
    /// `true` means received, `false` means not received.
    pub(crate) fn ack_vector(&self) -> Vec<(bool, u64)> {
        if self.entries.is_empty() {
            return Vec::new();
        }

        let mut runs = Vec::new();
        let mut current_state = self.entries[0].state == RecvEntryState::Received;
        let mut current_count: u64 = 1;

        for entry in self.entries.iter().skip(1) {
            let received = entry.state == RecvEntryState::Received;
            if received == current_state {
                current_count += 1;
            } else {
                runs.push((current_state, current_count));
                current_state = received;
                current_count = 1;
            }
        }
        runs.push((current_state, current_count));

        runs
    }

    /// The current base DataSeqNum (lowest unresolved).
    pub(crate) fn base_seq(&self) -> u64 {
        self.base_seq
    }

    /// The highest DataSeqNum received.
    pub(crate) fn highest_seq(&self) -> u64 {
        self.highest_seq
    }

    /// The next ChannelSeqNum expected for in-order delivery.
    pub(crate) fn next_channel_seq(&self) -> u64 {
        self.next_channel_seq
    }

    /// Whether there are gaps in the received data (some pending slots
    /// exist between base and highest).
    pub(crate) fn has_gaps(&self) -> bool {
        self.entries.iter().any(|e| e.state == RecvEntryState::Pending)
    }

    /// Number of entries in the reorder buffer waiting to be delivered.
    #[cfg(test)]
    pub(crate) fn reorder_buf_len(&self) -> usize {
        self.reorder_buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_window_state() {
        let w = RecvWindow::new(1, 100, 6);
        assert_eq!(w.base_seq(), 1);
        assert_eq!(w.next_channel_seq(), 100);
        assert!(!w.has_gaps());
        assert_eq!(w.reorder_buf_len(), 0);
    }

    #[test]
    fn receive_in_order() {
        let mut w = RecvWindow::new(1, 100, 6);

        assert!(w.receive(1, 100, vec![0xAA]));
        assert!(w.receive(2, 101, vec![0xBB]));
        assert!(w.receive(3, 102, vec![0xCC]));

        let delivered = w.drain_ordered();
        assert_eq!(delivered.len(), 3);
        assert_eq!(delivered[0], vec![0xAA]);
        assert_eq!(delivered[1], vec![0xBB]);
        assert_eq!(delivered[2], vec![0xCC]);
        assert_eq!(w.next_channel_seq(), 103);
    }

    #[test]
    fn receive_out_of_order() {
        let mut w = RecvWindow::new(1, 100, 6);

        // Receive seq 3 first (out of order)
        assert!(w.receive(3, 102, vec![0xCC]));
        let delivered = w.drain_ordered();
        assert!(delivered.is_empty()); // can't deliver until 100 arrives

        // Receive seq 1
        assert!(w.receive(1, 100, vec![0xAA]));
        let delivered = w.drain_ordered();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0], vec![0xAA]);

        // Receive seq 2 → now 101 and 102 are contiguous
        assert!(w.receive(2, 101, vec![0xBB]));
        let delivered = w.drain_ordered();
        assert_eq!(delivered.len(), 2);
        assert_eq!(delivered[0], vec![0xBB]);
        assert_eq!(delivered[1], vec![0xCC]);
    }

    #[test]
    fn reject_duplicate() {
        let mut w = RecvWindow::new(1, 100, 6);

        assert!(w.receive(1, 100, vec![0xAA]));
        // Duplicate
        assert!(!w.receive(1, 100, vec![0xAA]));
    }

    #[test]
    fn reject_below_base() {
        let mut w = RecvWindow::new(5, 100, 6);

        // data_seq 3 is below base_seq 5
        assert!(!w.receive(3, 98, vec![0x00]));
    }

    #[test]
    fn reject_beyond_window() {
        let mut w = RecvWindow::new(1, 100, 2); // window = 4

        // data_seq 5 is at index 4 (>= max_entries=4)
        assert!(!w.receive(5, 104, vec![0x00]));

        // data_seq 4 is at index 3 (last valid slot)
        assert!(w.receive(4, 103, vec![0x00]));
    }

    #[test]
    fn has_gaps_detection() {
        let mut w = RecvWindow::new(1, 100, 6);

        w.receive(1, 100, vec![0x00]);
        w.receive(3, 102, vec![0x00]); // gap at seq 2

        assert!(w.has_gaps());
    }

    #[test]
    fn ack_vector_contiguous() {
        let mut w = RecvWindow::new(1, 100, 6);

        w.receive(1, 100, vec![0x00]);
        w.receive(2, 101, vec![0x00]);
        w.receive(3, 102, vec![0x00]);

        let ack_vec = w.ack_vector();
        assert_eq!(ack_vec, vec![(true, 3)]);
    }

    #[test]
    fn ack_vector_with_gap() {
        let mut w = RecvWindow::new(1, 100, 6);

        w.receive(1, 100, vec![0x00]);
        w.receive(3, 102, vec![0x00]); // gap at index 1 (data_seq 2)

        let ack_vec = w.ack_vector();
        // entries[0] = Received, entries[1] = Pending, entries[2] = Received
        assert_eq!(ack_vec, vec![(true, 1), (false, 1), (true, 1)]);
    }

    #[test]
    fn ack_vector_empty() {
        let w = RecvWindow::new(1, 100, 6);
        assert!(w.ack_vector().is_empty());
    }

    /// The buffer is bounded, but the packet that empties it is exempt.
    ///
    /// A cap of "full, and not already buffered" rejects precisely the
    /// ChannelSeqNum the buffer is waiting on, because that one is by
    /// definition not in it. Reassembly then never restarts.
    #[test]
    fn the_packet_that_drains_the_buffer_is_never_refused() {
        let window = 1usize << 4;
        let mut w = RecvWindow::new(1, 1, 4);
        let mut data_seq = 1u64;

        // ChannelSeqNum 1 goes missing. Everything after it arrives, and each
        // acknowledgment slides the window on, so the buffer fills to its cap.
        for channel_seq in 2..=u64::try_from(window * 2).expect("window fits in u64") {
            w.receive(data_seq, channel_seq, vec![0xAB]);
            w.release_acknowledged();
            data_seq += 1;
        }

        assert_eq!(w.reorder_buf_len(), window, "the buffer should be at its cap");
        assert!(w.drain_ordered().is_empty(), "nothing is deliverable yet");

        // The sender retransmits the missing datum. [MS-RDPEUDP2] gives the
        // retransmission a fresh DataSeqNum and keeps the ChannelSeqNum.
        assert!(
            w.receive(w.base_seq(), 1, vec![0x01]),
            "the packet that drains the buffer was refused"
        );

        assert_eq!(
            w.drain_ordered().len(),
            window + 1,
            "the missing datum should release everything behind it"
        );
    }

    /// Data the application has already seen must not take a buffer slot.
    ///
    /// `drain_ordered` only removes at `next_channel_seq`, so an entry below
    /// it is never collected and holds its slot for the rest of the
    /// connection.
    #[test]
    fn a_redelivered_channel_seq_is_acknowledged_but_not_buffered() {
        let mut w = RecvWindow::new(1, 1, 4);

        assert!(w.receive(1, 1, vec![0xAA]));
        assert_eq!(w.drain_ordered(), vec![vec![0xAA]]);
        assert_eq!(w.reorder_buf_len(), 0);

        // The sender did not see our acknowledgment and resent the datum under
        // a new DataSeqNum.
        assert!(
            w.receive(2, 1, vec![0xAA]),
            "the duplicate should still be acknowledged"
        );
        assert_eq!(w.reorder_buf_len(), 0, "and it should not occupy a slot");
        assert!(w.drain_ordered().is_empty());
    }

    /// The reorder buffer is bounded by the window. A peer sending packets
    /// whose ChannelSeqNum never lets the buffer drain would otherwise add one
    /// entry per datagram for as long as it cared to keep sending.
    #[test]
    fn reorder_buffer_is_bounded_by_the_window() {
        let window = 1usize << 4;
        let mut w = RecvWindow::new(1, 1, 4);

        // Every packet arrives except the one the buffer is waiting on, so
        // nothing can ever be delivered and nothing drains.
        for offset in 0..window * 4 {
            let seq = 2 + u64::try_from(offset).expect("offset fits in u64");
            w.advance_base(seq);
            w.receive(seq, seq, vec![0xAB]);
        }

        assert!(w.reorder_buf_len() <= window, "the reorder buffer grew past the window");
    }

    #[test]
    fn advance_base() {
        let mut w = RecvWindow::new(1, 100, 6);

        w.receive(1, 100, vec![0x00]);
        w.receive(2, 101, vec![0x00]);
        w.receive(3, 102, vec![0x00]);
        w.receive(5, 104, vec![0x00]);

        // Advance base to 3 → entries for data_seq 1 and 2 are removed
        w.advance_base(3);
        assert_eq!(w.base_seq(), 3);

        // The ack vector should now start from data_seq 3
        let ack_vec = w.ack_vector();
        // entries[0] = seq3 (Received), entries[1] = seq4 (Pending), entries[2] = seq5 (Received)
        assert_eq!(ack_vec, vec![(true, 1), (false, 1), (true, 1)]);
    }

    #[test]
    fn advance_base_no_effect_if_not_advancing() {
        let mut w = RecvWindow::new(5, 100, 6);
        w.advance_base(3); // 3 < base_seq=5, no effect
        assert_eq!(w.base_seq(), 5);
    }

    #[test]
    fn highest_seq_tracking() {
        let mut w = RecvWindow::new(1, 100, 6);

        w.receive(3, 102, vec![0x00]);
        assert_eq!(w.highest_seq(), 3);

        w.receive(1, 100, vec![0x00]);
        assert_eq!(w.highest_seq(), 3); // still 3

        w.receive(5, 104, vec![0x00]);
        assert_eq!(w.highest_seq(), 5);
    }

    #[test]
    fn retransmit_different_data_seq_same_channel_seq() {
        let mut w = RecvWindow::new(1, 100, 6);

        // Original: data_seq=1, channel_seq=100
        w.receive(1, 100, vec![0xAA]);
        let delivered = w.drain_ordered();
        assert_eq!(delivered, vec![vec![0xAA]]);

        // Retransmit arrives: data_seq=5, channel_seq=100 (same)
        // Should be rejected as duplicate channel_seq (data already delivered)
        // Actually, data_seq=5 is a new DataSeqNum slot, but channel_seq=100
        // is already past next_channel_seq. The receive window records it
        // by data_seq, and the reorder buf records by channel_seq.
        // Since channel_seq 100 was already drained, this won't double-deliver.
        assert!(w.receive(5, 100, vec![0xBB]));

        // drain_ordered won't return anything because next_channel_seq is 101
        // and channel_seq 100 was re-inserted but is behind
        let delivered = w.drain_ordered();
        assert!(delivered.is_empty());
    }

    #[test]
    fn reorder_buf_len() {
        let mut w = RecvWindow::new(1, 100, 6);

        w.receive(3, 102, vec![0x00]); // buffered
        w.receive(5, 104, vec![0x00]); // buffered
        assert_eq!(w.reorder_buf_len(), 2);

        w.receive(1, 100, vec![0x00]);
        w.drain_ordered(); // delivers 100
        assert_eq!(w.reorder_buf_len(), 2); // 102 and 104 still buffered
    }

    #[test]
    fn large_gap_then_fill() {
        let mut w = RecvWindow::new(1, 100, 6);

        // Receive packets 1, 5, 10 (with gaps)
        w.receive(1, 100, vec![0x01]);
        w.receive(5, 104, vec![0x05]);
        w.receive(10, 109, vec![0x0A]);

        let delivered = w.drain_ordered();
        assert_eq!(delivered.len(), 1); // only 100

        // Fill the gaps progressively
        for i in 2..=4 {
            w.receive(i, 99 + i, vec![u8::try_from(i).expect("fits")]);
        }
        let delivered = w.drain_ordered();
        assert_eq!(delivered.len(), 4); // 101, 102, 103, 104

        // Fill 105-108
        for i in 6..=9 {
            w.receive(i, 99 + i, vec![u8::try_from(i).expect("fits")]);
        }
        let delivered = w.drain_ordered();
        assert_eq!(delivered.len(), 5); // 105, 106, 107, 108, 109
    }
}
