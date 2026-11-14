//! Server-side Heartbeat PDU emission (MS-RDPBCGR 2.2.16).

/// Configuration for periodic Server Heartbeat PDUs (MS-RDPBCGR 2.2.16.1).
///
/// Heartbeats let the client monitor connection health in real time: when it
/// stops receiving them (and everything else) for long enough, it can warn
/// the user or start a reconnect attempt. They ride the MCS message channel
/// and are only sent while the connection is otherwise idle; ordinary
/// traffic doubles as the liveness signal.
///
/// The specification mandates no particular values, so the defaults are this
/// library's own choice: a 5-second period keeps idle-link detection
/// responsive at negligible traffic cost (8 bytes of payload per interval),
/// `warning_count` = 3 puts a client-side warning at roughly 15 seconds of
/// silence, and `reconnect_count` = 5 a reconnect attempt at roughly
/// 40 seconds. The client MAY ignore both counts (2.2.16.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeartbeatConfig {
    /// Seconds between heartbeats while the connection is otherwise idle.
    ///
    /// `0` is treated as `1`.
    pub period_secs: u8,
    /// Missed heartbeats that SHOULD trigger a client-side warning.
    pub warning_count: u8,
    /// Missed heartbeats after the warning that SHOULD trigger a client-side
    /// reconnect attempt.
    pub reconnect_count: u8,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            period_secs: 5,
            warning_count: 3,
            reconnect_count: 5,
        }
    }
}

/// Decides at each heartbeat tick whether the interval that just ended was idle.
///
/// A heartbeat is only sent when no other PDU was sent during the interval
/// (MS-RDPBCGR 2.2.16.1), so the heartbeat's own write is not traffic for the
/// next interval.
#[derive(Debug)]
pub(crate) struct HeartbeatGate {
    writes_at_last_tick: u64,
}

impl HeartbeatGate {
    /// Starts the first interval at `write_count` successful writes.
    pub(crate) fn new(write_count: u64) -> Self {
        Self {
            writes_at_last_tick: write_count,
        }
    }

    /// Ends an interval at `write_count` successful writes and returns whether
    /// to send a heartbeat.
    ///
    /// When it returns `true`, the caller makes exactly one write for the
    /// heartbeat before the next tick, or ends the connection if that write
    /// fails.
    pub(crate) fn tick(&mut self, write_count: u64) -> bool {
        let idle = write_count == self.writes_at_last_tick;
        self.writes_at_last_tick = write_count.wrapping_add(u64::from(idle));
        idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // oracle: spec MS-RDPBCGR §2.2.16.1 sends a heartbeat when no other PDU was sent in the interval.
    #[test]
    fn a_heartbeat_does_not_count_as_traffic_for_the_next_interval() {
        let mut gate = HeartbeatGate::new(7);
        assert!(gate.tick(7));
        // Only the heartbeat was written since.
        assert!(gate.tick(8));
    }

    // oracle: spec MS-RDPBCGR §2.2.16.1 sends a heartbeat when no other PDU was sent in the interval.
    #[test]
    fn an_interval_with_other_traffic_sends_no_heartbeat() {
        let mut gate = HeartbeatGate::new(0);
        assert!(!gate.tick(3));
        assert!(gate.tick(3));
    }
}
