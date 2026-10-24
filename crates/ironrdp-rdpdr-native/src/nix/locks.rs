//! Byte-range locks the server takes on redirected files, as [MS-FSA] 2.1.4.10, 2.1.5.8 and 2.1.5.9 describe.
//!
//! The locks bind the server's own handles to each other; local processes do not see them.
//!
//! [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/124bb289-eeef-4653-b9c6-4fb93dd07a21

use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{ClientDriveLockControlResponse, DeviceIoRequest, NtStatus, RdpLockInfo};
use ironrdp_svc::SvcMessage;

use super::backend::FileIdentity;

/// The most locks the server may hold at once, so it cannot grow the table without bound.
const MAX_LOCKS: usize = 65_536;

/// The most lock requests that may wait at once, so the server cannot queue them without bound.
const MAX_WAITING: usize = 1_024;

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
    /// INVARIANT: holds at most `MAX_WAITING` entries.
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
    /// Whether accessing a range conflicts with the locks on the file, as [MS-FSA] 2.1.4.10 decides.
    ///
    /// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/124bb289-eeef-4653-b9c6-4fb93dd07a21
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
                if MAX_WAITING <= self.waiting.len() {
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
                return (range_not_locked(), self.grant_waiting());
            };
            self.locks.swap_remove(index);
        }
        (NtStatus::SUCCESS, self.grant_waiting())
    }

    /// Releases every lock `owner` holds and cancels its waiting requests, as closing a handle does.
    ///
    /// Returns the responses to the cancelled requests and to other waiting requests that can now proceed.
    pub(crate) fn close(&mut self, owner: u32) -> Vec<SvcMessage> {
        self.locks.retain(|lock| lock.owner != owner);
        let (cancelled, waiting) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition::<Vec<_>, _>(|waiting| waiting.request.file_id == owner);
        self.waiting = waiting;
        let mut responses = cancelled
            .into_iter()
            .map(|waiting| response(waiting.request, NtStatus::CANCELLED))
            .collect::<Vec<_>>();
        responses.extend(self.grant_waiting());
        responses
    }

    /// Forgets every lock and waiting request, as the server does when the channel restarts.
    pub(crate) fn clear(&mut self) {
        self.locks.clear();
        self.waiting.clear();
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

    /// Grants the waiting requests that no longer conflict, oldest first, and returns their responses.
    fn grant_waiting(&mut self) -> Vec<SvcMessage> {
        let mut responses = Vec::new();
        let mut index = 0;
        while index < self.waiting.len() {
            let waiting = &self.waiting[index];
            let (owner, identity, ranges, exclusive) = (
                waiting.request.file_id,
                waiting.identity,
                waiting.ranges.clone(),
                waiting.exclusive,
            );
            let granted = self.try_lock(owner, identity, &ranges, exclusive).is_ok();
            if granted {
                let waiting = self.waiting.remove(index);
                responses.push(response(waiting.request, NtStatus::SUCCESS));
            } else {
                index += 1;
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

/// Refuses an empty request and a range past the largest offset, as [MS-FSA] 2.1.5.8 and 2.1.5.9 require.
fn validate(ranges: &[RdpLockInfo]) -> Result<(), NtStatus> {
    if ranges.is_empty() {
        return Err(NtStatus::INVALID_PARAMETER);
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
