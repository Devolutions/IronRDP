//! Byte-range locks the server takes on redirected files, as [2.1.4.10] Algorithm for Determining If a Range Access Conflicts with Byte-Range Locks, [2.1.5.8] Server Requests a Byte-Range Lock and [2.1.5.9] Server Requests an Unlock of a Byte-Range describe in MS-FSA.
//!
//! The locks bind the server's own handles to each other; local processes do not see them.
//!
//! [2.1.4.10]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/124bb289-eeef-4653-b9c6-4fb93dd07a21
//! [2.1.5.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/369103ec-a8af-452b-8006-aff07b925b61
//! [2.1.5.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/84e68de6-31a6-4cba-afd4-00b1bac6d1a2

use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{ClientDriveLockControlResponse, DeviceIoRequest, NtStatus, RdpLockInfo};
use ironrdp_svc::SvcMessage;

use super::backend::FileIdentity;

/// The most locks the server may hold at once, so it cannot grow the table without bound.
const MAX_LOCKS: usize = 65_536;

/// The most ranges one lock or unlock request may carry.
///
/// Windows sends one range per lock request.
/// Every range is checked against every held lock, so the cap bounds the work one request causes.
const MAX_RANGES: usize = 64;

/// The most ranges the waiting requests may carry together.
///
/// Every unlock and close retries every waiting request, so the cap bounds both the queue and the work each release causes.
const MAX_WAITING_RANGES: usize = 256;

/// STATUS_FILE_LOCK_CONFLICT, which [`NtStatus`] does not name.
pub(crate) fn file_lock_conflict() -> NtStatus {
    NtStatus::from(0xC000_0054)
}

/// STATUS_RANGE_NOT_LOCKED, which [`NtStatus`] does not name.
fn range_not_locked() -> NtStatus {
    NtStatus::from(0xC000_007E)
}

/// STATUS_INVALID_LOCK_RANGE, which [`NtStatus`] does not name.
fn invalid_lock_range() -> NtStatus {
    NtStatus::from(0xC000_001E)
}

#[derive(Debug, Default)]
pub(crate) struct ByteRangeLocks {
    /// INVARIANT: holds at most `MAX_LOCKS` entries.
    locks: Vec<ByteRangeLock>,
    /// Requests waiting for conflicting locks to go away, oldest first.
    ///
    /// INVARIANT: the entries carry at most `MAX_WAITING_RANGES` ranges together.
    waiting: Vec<WaitingLock>,
}

#[derive(Debug)]
struct ByteRangeLock {
    identity: FileIdentity,
    /// The file ID of the handle that holds the lock.
    owner: u32,
    offset: u64,
    length: u64,
    exclusive: bool,
}

#[derive(Debug)]
struct WaitingLock {
    request: DeviceIoRequest,
    identity: FileIdentity,
    ranges: Vec<RdpLockInfo>,
    exclusive: bool,
}

impl ByteRangeLocks {
    /// Whether accessing a range conflicts with the locks on the file, as MS-FSA [2.1.4.10] Algorithm for Determining If a Range Access Conflicts with Byte-Range Locks decides.
    ///
    /// [2.1.4.10]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/124bb289-eeef-4653-b9c6-4fb93dd07a21
    pub(crate) fn conflicts(
        &self,
        identity: FileIdentity,
        owner: u32,
        offset: u64,
        length: u64,
        exclusive: bool,
        lock_intent: bool,
    ) -> bool {
        if offset == 0 && length == 0 {
            return false;
        }
        let last = last_byte(offset, length);
        self.locks.iter().filter(|lock| lock.identity == identity).any(|lock| {
            let overlaps = !(lock.offset == 0 && lock.length == 0)
                && offset <= last_byte(lock.offset, lock.length)
                && lock.offset <= last;
            if !overlaps {
                return false;
            }
            if lock.exclusive {
                // Overlapping exclusive locks are refused even to their owner.
                lock.owner != owner || (exclusive && lock_intent)
            } else {
                // A shared lock blocks every access with exclusive intent, its owner's included.
                exclusive
            }
        })
    }

    /// Takes `ranges` for `owner`, either all of them or none.
    ///
    /// A conflict fails with STATUS_LOCK_NOT_GRANTED, or queues the request when `wait` is set and returns `None`.
    pub(crate) fn lock(
        &mut self,
        request: &DeviceIoRequest,
        identity: FileIdentity,
        ranges: &[RdpLockInfo],
        exclusive: bool,
        wait: bool,
    ) -> Option<NtStatus> {
        if let Err(status) = validate(ranges) {
            return Some(status);
        }
        match self.try_lock(request.file_id, identity, ranges, exclusive) {
            Ok(()) => Some(NtStatus::SUCCESS),
            Err(status) if status == NtStatus::LOCK_NOT_GRANTED && wait => {
                let waiting_ranges = self.waiting.iter().map(|waiting| waiting.ranges.len()).sum::<usize>();
                if MAX_WAITING_RANGES < waiting_ranges.saturating_add(ranges.len()) {
                    return Some(NtStatus::INSUFFICIENT_RESOURCES);
                }
                self.waiting.push(WaitingLock {
                    request: request.clone(),
                    identity,
                    ranges: ranges.to_vec(),
                    exclusive,
                });
                None
            }
            Err(status) => Some(status),
        }
    }

    /// Releases `ranges` held by `owner`, then returns the responses to waiting requests that can now proceed.
    pub(crate) fn unlock(
        &mut self,
        owner: u32,
        identity: FileIdentity,
        ranges: &[RdpLockInfo],
    ) -> (NtStatus, Vec<SvcMessage>) {
        if let Err(status) = validate(ranges) {
            return (status, Vec::new());
        }
        // Ranges released before an unmatched one stay released.
        let mut released = false;
        for range in ranges {
            // [MS-FSA] 2.1.5.9 releases an exact match, preferring an exclusive lock over a shared one.
            let matching = |lock: &ByteRangeLock| {
                lock.identity == identity
                    && lock.owner == owner
                    && lock.offset == range.offset
                    && lock.length == range.length
            };
            let index = self
                .locks
                .iter()
                .position(|lock| matching(lock) && lock.exclusive)
                .or_else(|| self.locks.iter().rposition(matching));
            let Some(index) = index else {
                let granted = if released {
                    self.grant_waiting(identity)
                } else {
                    Vec::new()
                };
                return (range_not_locked(), granted);
            };
            self.locks.swap_remove(index);
            released = true;
        }
        (NtStatus::SUCCESS, self.grant_waiting(identity))
    }

    /// Releases every lock `owner` holds and cancels its waiting requests, as closing a handle does.
    ///
    /// Returns the responses to the cancelled requests and to other waiting requests that can now proceed.
    pub(crate) fn close(&mut self, owner: u32) -> Vec<SvcMessage> {
        // INVARIANT: a handle holds locks on one file only, so the last released identity is the only one.
        let mut released = None;
        self.locks.retain(|lock| {
            let is_owned = lock.owner == owner;
            if is_owned {
                released = Some(lock.identity);
            }
            !is_owned
        });
        let mut responses = self
            .waiting
            .extract_if(.., |waiting| waiting.request.file_id == owner)
            .map(|waiting| response(waiting.request, NtStatus::CANCELLED))
            .collect::<Vec<_>>();
        if let Some(identity) = released {
            responses.extend(self.grant_waiting(identity));
        }
        responses
    }

    fn try_lock(
        &mut self,
        owner: u32,
        identity: FileIdentity,
        ranges: &[RdpLockInfo],
        exclusive: bool,
    ) -> Result<(), NtStatus> {
        let first_added = self.locks.len();
        for range in ranges {
            let status = if self.conflicts(identity, owner, range.offset, range.length, exclusive, true) {
                Some(NtStatus::LOCK_NOT_GRANTED)
            } else if MAX_LOCKS <= self.locks.len() {
                Some(NtStatus::INSUFFICIENT_RESOURCES)
            } else {
                None
            };
            if let Some(status) = status {
                // A request is granted whole or not at all.
                self.locks.truncate(first_added);
                return Err(status);
            }
            self.locks.push(ByteRangeLock {
                identity,
                owner,
                offset: range.offset,
                length: range.length,
                exclusive,
            });
        }
        Ok(())
    }

    /// Grants the waiting requests on `identity` that no longer conflict, oldest first, and returns their responses.
    ///
    /// UNTESTED: waiting requests are retried only after a lock on their file is released.
    /// Each retry checks every waiting range against every held lock, so retrying on every close or failed unlock would let a server make each small request cost that much.
    /// The responses are the same either way, so after changing a caller, check that it calls this only when it released a lock, with that lock's file.
    fn grant_waiting(&mut self, identity: FileIdentity) -> Vec<SvcMessage> {
        let mut responses = Vec::new();
        // `try_lock` touches only the locks, so the queue can be taken out and refilled in order with what stays.
        for waiting in std::mem::take(&mut self.waiting) {
            let is_granted = waiting.identity == identity
                && self
                    .try_lock(waiting.request.file_id, identity, &waiting.ranges, waiting.exclusive)
                    .is_ok();
            if is_granted {
                responses.push(response(waiting.request, NtStatus::SUCCESS));
            } else {
                self.waiting.push(waiting);
            }
        }
        responses
    }
}

pub(crate) fn response(request: DeviceIoRequest, status: NtStatus) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::ClientDriveLockControlResponse(
        ClientDriveLockControlResponse::new(request, status),
    ))
}

/// Refuses an empty request and a range past the largest offset, as MS-FSA [2.1.5.8] Server Requests a Byte-Range Lock and [2.1.5.9] Server Requests an Unlock of a Byte-Range require, and more ranges than `MAX_RANGES`.
///
/// [2.1.5.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/369103ec-a8af-452b-8006-aff07b925b61
/// [2.1.5.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/84e68de6-31a6-4cba-afd4-00b1bac6d1a2
fn validate(ranges: &[RdpLockInfo]) -> Result<(), NtStatus> {
    if ranges.is_empty() {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    if MAX_RANGES < ranges.len() {
        return Err(NtStatus::INSUFFICIENT_RESOURCES);
    }
    if ranges
        .iter()
        .any(|range| range.length != 0 && range.offset.checked_add(range.length - 1).is_none())
    {
        return Err(invalid_lock_range());
    }
    Ok(())
}

/// The offset of the last byte in a range, saturating for a range that reaches past the largest offset.
///
/// A zero-length range ends before it starts, so it overlaps nothing that starts at or after its offset.
fn last_byte(offset: u64, length: u64) -> u64 {
    match length.checked_sub(1) {
        Some(extent) => offset.saturating_add(extent),
        None => offset.wrapping_sub(1),
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_rdpdr::pdu::efs::{MajorFunction, MinorFunction};

    use super::*;

    fn identity() -> FileIdentity {
        FileIdentity::of(&nix::sys::stat::stat("/").expect("stat the root directory"))
    }

    fn request(file_id: u32) -> DeviceIoRequest {
        DeviceIoRequest {
            device_id: 1,
            file_id,
            completion_id: 0,
            major_function: MajorFunction::LockControl,
            minor_function: MinorFunction::from(0),
        }
    }

    fn range(offset: u64, length: u64) -> RdpLockInfo {
        RdpLockInfo { length, offset }
    }

    // oracle: mutant lock cap check removed
    #[test]
    fn a_full_table_refuses_another_lock() {
        let mut table = ByteRangeLocks {
            locks: (1..MAX_LOCKS)
                .map(|index| ByteRangeLock {
                    identity: identity(),
                    owner: 1,
                    offset: u64::try_from(index).expect("index fits u64"),
                    length: 1,
                    exclusive: false,
                })
                .collect(),
            ..ByteRangeLocks::default()
        };

        let at_cap = table.lock(&request(2), identity(), &[range(u64::MAX - 1, 1)], false, false);
        let over_cap = table.lock(&request(2), identity(), &[range(u64::MAX - 2, 1)], false, false);

        assert_eq!(at_cap, Some(NtStatus::SUCCESS));
        assert_eq!(over_cap, Some(NtStatus::INSUFFICIENT_RESOURCES));
        assert_eq!(table.locks.len(), MAX_LOCKS);
    }

    // oracle: mutant waiting cap check removed
    #[test]
    fn a_full_queue_refuses_another_waiting_request() {
        let mut table = ByteRangeLocks::default();
        assert_eq!(
            table.lock(&request(1), identity(), &[range(0, 10)], true, false),
            Some(NtStatus::SUCCESS)
        );
        for _ in 1..MAX_WAITING_RANGES {
            assert_eq!(table.lock(&request(2), identity(), &[range(0, 10)], true, true), None);
        }

        // One range is left, so a request with two cannot wait and a request with one still can.
        let over_cap = table.lock(&request(2), identity(), &[range(0, 10), range(5, 1)], true, true);
        let at_cap = table.lock(&request(2), identity(), &[range(0, 10)], true, true);

        assert_eq!(over_cap, Some(NtStatus::INSUFFICIENT_RESOURCES));
        assert_eq!(at_cap, None);
        assert_eq!(table.waiting.len(), MAX_WAITING_RANGES);
    }

    // oracle: mutant per-request range cap removed
    #[test]
    fn a_request_with_too_many_ranges_is_refused() {
        let mut table = ByteRangeLocks::default();
        let ranges = (0..=MAX_RANGES)
            .map(|index| range(u64::try_from(index).expect("index fits u64"), 1))
            .collect::<Vec<_>>();
        let (at_cap, over_cap) = (&ranges[..MAX_RANGES], ranges.as_slice());

        let locked_over_cap = table.lock(&request(1), identity(), over_cap, true, false);
        let locked_at_cap = table.lock(&request(1), identity(), at_cap, true, false);
        let (unlocked_over_cap, _) = table.unlock(1, identity(), over_cap);
        let (unlocked_at_cap, _) = table.unlock(1, identity(), at_cap);

        assert_eq!(locked_over_cap, Some(NtStatus::INSUFFICIENT_RESOURCES));
        assert_eq!(locked_at_cap, Some(NtStatus::SUCCESS));
        assert_eq!(unlocked_over_cap, NtStatus::INSUFFICIENT_RESOURCES);
        assert_eq!(unlocked_at_cap, NtStatus::SUCCESS);
        assert!(table.locks.is_empty());
    }

    // oracle: mutant rollback of the ranges taken before a conflict removed
    #[test]
    fn a_refused_request_takes_none_of_its_ranges() {
        let mut table = ByteRangeLocks::default();
        assert_eq!(
            table.lock(&request(1), identity(), &[range(5, 1)], true, false),
            Some(NtStatus::SUCCESS)
        );

        let status = table.lock(&request(2), identity(), &[range(0, 1), range(5, 1)], true, false);

        assert_eq!(status, Some(NtStatus::LOCK_NOT_GRANTED));
        assert!(!table.conflicts(identity(), 3, 0, 1, true, true));
    }
}
