//! Deferred Windows RDPDR IRP completion and cancellation bookkeeping.

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use super::handles::{DirectoryChange, FileHandle};
use super::locks::lock_error_status;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    ClientDriveLockControlResponse, ClientDriveNotifyChangeDirectoryResponse, DeviceIoRequest, NtStatus,
    ServerDriveNotifyChangeDirectoryRequest,
};
use ironrdp_svc::SvcMessage;

const MAX_DEFERRED_OPERATIONS: usize = 128;
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Debug)]
pub(super) struct DeferredOperations {
    next_operation_id: u64,
    epoch: u64,
    operations: HashMap<u64, PendingLockOperation>,
    notifications: HashMap<u64, PendingDirectoryNotification>,
    completions: Arc<Mutex<Vec<DeferredCompletion>>>,
}

#[derive(Debug)]
struct PendingLockOperation {
    device_id: u32,
    file_id: u32,
    request: DeviceIoRequest,
    ranges: Vec<(u64, u64)>,
    handle: FileHandle,
    cancellation: Arc<LockCancellation>,
    worker: thread::JoinHandle<()>,
}

#[derive(Debug)]
struct LockCancellation {
    cancelled: AtomicBool,
    acquired: AtomicBool,
    released: AtomicBool,
}

#[derive(Debug)]
struct PendingDirectoryNotification {
    device_id: u32,
    file_id: u32,
    request: DeviceIoRequest,
    handle: Arc<FileHandle>,
    cancellation: Arc<DirectoryCancellation>,
    worker: thread::JoinHandle<()>,
}

#[derive(Debug)]
struct DirectoryCancellation {
    cancelled: AtomicBool,
    finished: AtomicBool,
}

struct DirectoryWorkerFinished<'a>(&'a DirectoryCancellation);

impl Drop for DirectoryWorkerFinished<'_> {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
    }
}

#[derive(Debug)]
struct DeferredCompletion {
    operation_id: u64,
    epoch: u64,
    message: SvcMessage,
}

impl DeferredOperations {
    pub(super) fn new() -> Self {
        Self {
            next_operation_id: 1,
            epoch: 0,
            operations: HashMap::new(),
            notifications: HashMap::new(),
            completions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn schedule_waiting_lock(
        &mut self,
        request: DeviceIoRequest,
        ranges: Vec<(u64, u64)>,
        exclusive: bool,
        handle: FileHandle,
    ) -> Result<(), NtStatus> {
        if self.len() == MAX_DEFERRED_OPERATIONS {
            return Err(NtStatus::UNSUCCESSFUL);
        }

        let operation_id = self.allocate_operation_id()?;
        let cancellation = Arc::new(LockCancellation {
            cancelled: AtomicBool::new(false),
            acquired: AtomicBool::new(false),
            released: AtomicBool::new(false),
        });
        let cancellation_handle = handle.try_clone().map_err(|_| NtStatus::UNSUCCESSFUL)?;
        let completions = Arc::clone(&self.completions);
        let epoch = self.epoch;
        let worker_request = request.clone();
        let worker_ranges = ranges.clone();
        let worker_cancellation = Arc::clone(&cancellation);
        let worker = thread::Builder::new()
            .name("ironrdp-rdpdr-lock".to_owned())
            .spawn(move || {
                wait_for_lock(
                    operation_id,
                    epoch,
                    worker_request,
                    worker_ranges,
                    exclusive,
                    handle,
                    worker_cancellation,
                    completions,
                )
            })
            .map_err(|_| NtStatus::UNSUCCESSFUL)?;
        let previous = self.operations.insert(
            operation_id,
            PendingLockOperation {
                device_id: request.device_id,
                file_id: request.file_id,
                request,
                ranges,
                handle: cancellation_handle,
                cancellation,
                worker,
            },
        );
        debug_assert!(previous.is_none(), "deferred RDPDR operation IDs never collide");

        Ok(())
    }

    pub(super) fn schedule_directory_notification(
        &mut self,
        request: ServerDriveNotifyChangeDirectoryRequest,
        handle: FileHandle,
    ) -> Result<(), NtStatus> {
        if self.len() == MAX_DEFERRED_OPERATIONS {
            return Err(NtStatus::UNSUCCESSFUL);
        }

        let operation_id = self.allocate_operation_id()?;
        let cancellation = Arc::new(DirectoryCancellation {
            cancelled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        });
        let completions = Arc::clone(&self.completions);
        let epoch = self.epoch;
        let worker_request = request.device_io_request.clone();
        let worker_cancellation = Arc::clone(&cancellation);
        let watch_tree = request.watch_tree != 0;
        let completion_filter = request.completion_filter;
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let directory_change = handle
            .begin_directory_changes(watch_tree, completion_filter)
            .map_err(|error| lock_error_status(&error))?;
        let notification_handle = directory_change.cancellation_handle();
        let worker = thread::Builder::new()
            .name("ironrdp-rdpdr-notify".to_owned())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    wait_for_directory_change(
                        operation_id,
                        epoch,
                        worker_request,
                        directory_change,
                        worker_cancellation,
                        completions,
                    )
                })
            })
            .map_err(|_| NtStatus::UNSUCCESSFUL)?;
        let previous = self.notifications.insert(
            operation_id,
            PendingDirectoryNotification {
                device_id: request.device_io_request.device_id,
                file_id: request.device_io_request.file_id,
                request: request.device_io_request,
                handle: notification_handle,
                cancellation,
                worker,
            },
        );
        debug_assert!(previous.is_none(), "deferred RDPDR operation IDs never collide");
        Ok(())
    }

    pub(super) fn poll(&mut self) -> Vec<SvcMessage> {
        let completions = {
            let mut completions = self
                .completions
                .lock()
                .expect("deferred RDPDR completion queue mutex must not be poisoned");
            core::mem::take(&mut *completions)
        };
        if !completions.is_empty() {
            tracing::debug!(
                count = completions.len(),
                "Draining completed RDPDR deferred operations"
            );
        }

        completions
            .into_iter()
            .filter_map(|completion| {
                let is_pending = if let Some(operation) = self.operations.remove(&completion.operation_id) {
                    join_lock_worker(operation);
                    true
                } else if let Some(notification) = self.notifications.remove(&completion.operation_id) {
                    join_directory_worker(notification);
                    true
                } else {
                    false
                };
                (completion.epoch == self.epoch && is_pending).then_some(completion.message)
            })
            .collect()
    }

    pub(super) fn cancel_file(&mut self, device_id: u32, file_id: u32) -> Vec<SvcMessage> {
        let operation_ids = self
            .operations
            .iter()
            .filter_map(|(&operation_id, operation)| {
                (operation.device_id == device_id && operation.file_id == file_id).then_some(operation_id)
            })
            .collect::<Vec<_>>();
        let notification_ids = self
            .notifications
            .iter()
            .filter_map(|(&operation_id, notification)| {
                (notification.device_id == device_id && notification.file_id == file_id).then_some(operation_id)
            })
            .collect::<Vec<_>>();

        let mut cancelled = Vec::with_capacity(operation_ids.len() + notification_ids.len());
        for operation_id in &operation_ids {
            let operation = self
                .operations
                .remove(operation_id)
                .expect("operation ID was collected from the pending operation table");
            let request = operation.request.clone();
            cancel_lock_operation(operation);
            cancelled.push(lock_completion(request, NtStatus::CANCELLED));
        }
        for operation_id in &notification_ids {
            let notification = self
                .notifications
                .remove(operation_id)
                .expect("operation ID was collected from the pending notification table");
            let request = notification.request.clone();
            cancel_directory_notification(notification);
            cancelled.push(directory_completion(request, NtStatus::SUCCESS, Vec::new()));
        }
        self.discard_queued(&operation_ids);
        self.discard_queued(&notification_ids);
        cancelled
    }

    pub(super) fn cancel_drive(&mut self, device_id: u32) -> Vec<SvcMessage> {
        let operation_ids = self
            .operations
            .iter()
            .filter_map(|(&operation_id, operation)| (operation.device_id == device_id).then_some(operation_id))
            .collect::<Vec<_>>();
        let notification_ids = self
            .notifications
            .iter()
            .filter_map(|(&operation_id, notification)| (notification.device_id == device_id).then_some(operation_id))
            .collect::<Vec<_>>();

        let mut cancelled = Vec::with_capacity(operation_ids.len() + notification_ids.len());
        for operation_id in &operation_ids {
            let operation = self
                .operations
                .remove(operation_id)
                .expect("operation ID was collected from the pending operation table");
            let request = operation.request.clone();
            cancel_lock_operation(operation);
            cancelled.push(lock_completion(request, NtStatus::CANCELLED));
        }
        for operation_id in &notification_ids {
            let notification = self
                .notifications
                .remove(operation_id)
                .expect("operation ID was collected from the pending notification table");
            let request = notification.request.clone();
            cancel_directory_notification(notification);
            cancelled.push(directory_completion(request, NtStatus::CANCELLED, Vec::new()));
        }
        self.discard_queued(&operation_ids);
        self.discard_queued(&notification_ids);
        cancelled
    }

    pub(super) fn reset(&mut self) {
        self.cancel_matching(|_| true);
        self.cancel_notifications(|_| true);
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn cancel_matching(&mut self, predicate: impl Fn(&PendingLockOperation) -> bool) {
        let operation_ids = self
            .operations
            .iter()
            .filter_map(|(&operation_id, operation)| predicate(operation).then_some(operation_id))
            .collect::<Vec<_>>();
        for operation_id in &operation_ids {
            let operation = self
                .operations
                .remove(operation_id)
                .expect("operation ID was collected from the pending operation table");
            cancel_lock_operation(operation);
        }
        self.discard_queued(&operation_ids);
    }

    fn cancel_notifications(&mut self, predicate: impl Fn(&PendingDirectoryNotification) -> bool) {
        let operation_ids = self
            .notifications
            .iter()
            .filter_map(|(&operation_id, notification)| predicate(notification).then_some(operation_id))
            .collect::<Vec<_>>();
        for operation_id in &operation_ids {
            let notification = self
                .notifications
                .remove(operation_id)
                .expect("operation ID was collected from the pending notification table");
            cancel_directory_notification(notification);
        }
        self.discard_queued(&operation_ids);
    }

    fn allocate_operation_id(&mut self) -> Result<u64, NtStatus> {
        let operation_id = self.next_operation_id;
        self.next_operation_id = self.next_operation_id.checked_add(1).ok_or(NtStatus::UNSUCCESSFUL)?;
        Ok(operation_id)
    }

    fn len(&self) -> usize {
        self.operations.len() + self.notifications.len()
    }

    fn discard_queued(&self, operation_ids: &[u64]) {
        if operation_ids.is_empty() {
            return;
        }

        self.completions
            .lock()
            .expect("deferred RDPDR completion queue mutex must not be poisoned")
            .retain(|completion| !operation_ids.contains(&completion.operation_id));
    }
}

impl Drop for DeferredOperations {
    fn drop(&mut self) {
        self.cancel_matching(|_| true);
        self.cancel_notifications(|_| true);
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the worker requires the full deferred-operation ownership and completion context"
)]
fn wait_for_lock(
    operation_id: u64,
    epoch: u64,
    request: DeviceIoRequest,
    ranges: Vec<(u64, u64)>,
    exclusive: bool,
    handle: FileHandle,
    cancellation: Arc<LockCancellation>,
    completions: Arc<Mutex<Vec<DeferredCompletion>>>,
) {
    loop {
        if cancellation.cancelled.load(Ordering::Acquire) {
            return;
        }

        match handle.lock_ranges(&ranges, exclusive) {
            Ok(()) => {
                cancellation.acquired.store(true, Ordering::Release);
                if cancellation.cancelled.load(Ordering::Acquire) {
                    release_cancelled_lock(&handle, &ranges, &cancellation, request.file_id);
                    return;
                }

                queue_completion(
                    &completions,
                    DeferredCompletion {
                        operation_id,
                        epoch,
                        message: lock_completion(request, NtStatus::SUCCESS),
                    },
                );
                return;
            }
            Err(error) if lock_error_status(&error) == NtStatus::LOCK_NOT_GRANTED => {
                thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(error) => {
                if !cancellation.cancelled.load(Ordering::Acquire) {
                    queue_completion(
                        &completions,
                        DeferredCompletion {
                            operation_id,
                            epoch,
                            message: lock_completion(request, lock_error_status(&error)),
                        },
                    );
                }
                return;
            }
        }
    }
}

fn wait_for_directory_change(
    operation_id: u64,
    epoch: u64,
    request: DeviceIoRequest,
    directory_change: Box<DirectoryChange>,
    cancellation: Arc<DirectoryCancellation>,
    completions: Arc<Mutex<Vec<DeferredCompletion>>>,
) {
    let _finished = DirectoryWorkerFinished(&cancellation);
    if cancellation.cancelled.load(Ordering::Acquire) {
        tracing::debug!(operation_id, "Cancelled RDPDR directory notification before waiting");
        return;
    }

    tracing::debug!(operation_id, "Waiting for RDPDR directory notification");
    match directory_change.wait() {
        Ok(buffer) if !cancellation.cancelled.load(Ordering::Acquire) => queue_completion(
            &completions,
            DeferredCompletion {
                operation_id,
                epoch,
                message: directory_completion(request, NtStatus::SUCCESS, buffer),
            },
        ),
        Ok(_) => tracing::debug!(operation_id, "Discarded cancelled RDPDR directory notification"),
        Err(error) if !cancellation.cancelled.load(Ordering::Acquire) => {
            tracing::debug!(error = %error, "RDPDR directory notification failed");
            queue_completion(
                &completions,
                DeferredCompletion {
                    operation_id,
                    epoch,
                    message: directory_completion(request, lock_error_status(&error), Vec::new()),
                },
            );
        }
        Err(_) => tracing::debug!(operation_id, "Discarded cancelled RDPDR directory notification failure"),
    }
}

fn queue_completion(completions: &Mutex<Vec<DeferredCompletion>>, completion: DeferredCompletion) {
    tracing::debug!(
        operation_id = completion.operation_id,
        "Queued completed RDPDR deferred operation"
    );
    completions
        .lock()
        .expect("deferred RDPDR completion queue mutex must not be poisoned")
        .push(completion);
}

fn lock_completion(request: DeviceIoRequest, status: NtStatus) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::ClientDriveLockControlResponse(
        ClientDriveLockControlResponse::new(request, status),
    ))
}

fn directory_completion(request: DeviceIoRequest, status: NtStatus, buffer: Vec<u8>) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(
        ClientDriveNotifyChangeDirectoryResponse::new(request, status, buffer),
    ))
}

fn join_lock_worker(operation: PendingLockOperation) {
    join_worker(operation.worker, operation.file_id, "lock");
}

fn cancel_lock_operation(operation: PendingLockOperation) {
    operation.cancellation.cancelled.store(true, Ordering::Release);
    let file_id = operation.file_id;
    let ranges = &operation.ranges;
    let cancellation = &operation.cancellation;
    join_worker(operation.worker, file_id, "lock");
    release_cancelled_lock(&operation.handle, ranges, cancellation, file_id);
}

fn release_cancelled_lock(handle: &FileHandle, ranges: &[(u64, u64)], cancellation: &LockCancellation, file_id: u32) {
    if cancellation.acquired.load(Ordering::Acquire)
        && cancellation
            .released
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        && let Err(error) = handle.unlock_ranges(ranges)
    {
        tracing::warn!(error = %error, file_id, "Failed to release cancelled deferred RDPDR lock");
    }
}

fn join_directory_worker(notification: PendingDirectoryNotification) {
    join_worker(notification.worker, notification.file_id, "directory notification");
}

fn join_worker(worker: thread::JoinHandle<()>, file_id: u32, operation: &str) {
    if worker.join().is_err() {
        tracing::error!(file_id, operation, "Deferred RDPDR worker panicked");
    }
}

fn cancel_directory_notification(notification: PendingDirectoryNotification) {
    notification.cancellation.cancelled.store(true, Ordering::Release);

    while !notification.cancellation.finished.load(Ordering::Acquire) {
        // SAFETY: the worker owns the duplicated asynchronous directory handle
        // until it marks itself finished. Repeating the cancellation closes the
        // race where this thread requests cancellation before the I/O manager
        // has observed the just-submitted notification request.
        let _ = unsafe { windows::Win32::System::IO::CancelIoEx(notification.handle.as_raw(), None) };
        thread::sleep(Duration::from_millis(1));
    }
    join_directory_worker(notification);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_rdpdr::pdu::efs::{MajorFunction, MinorFunction};
    use windows::Wdk::Storage::FileSystem::FILE_OPEN;
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    use crate::windows::handles::{FileOpenOptions, RootDirectory};
    use crate::windows::path::RelativePath;

    #[test]
    fn cancelling_a_waiting_lock_completes_once_and_discards_a_late_worker_result() {
        let temporary_directory = std::env::temp_dir();
        let volume_root = temporary_directory
            .ancestors()
            .last()
            .expect("temporary directory has a volume root");
        let file_path = temporary_directory.join(format!(
            "ironrdp-rdpdr-deferred-lock-test-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
                .as_nanos()
        ));
        std::fs::write(&file_path, [0u8; 16]).expect("create temporary test file");

        let root = RootDirectory::open(volume_root).expect("open temporary volume root");
        let relative_path = file_path
            .strip_prefix(volume_root)
            .expect("temporary file is beneath its volume root");
        let relative_path =
            RelativePath::parse(&format!(r"\{}", relative_path.display())).expect("temporary file has a valid path");
        let options = FileOpenOptions {
            desired_access: 0x0010_0083,
            allocation_size: None,
            file_attributes: FILE_ATTRIBUTE_NORMAL.0,
            share_access: (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
            create_disposition: FILE_OPEN.0,
            create_options: 0x0000_0020,
        };
        let (locking_handle, _) = root
            .open_relative_file(&relative_path, options)
            .expect("open first temporary file handle");
        let (waiting_handle, _) = root
            .open_relative_file(&relative_path, options)
            .expect("open second temporary file handle");
        locking_handle
            .lock_range(0, 8, true)
            .expect("acquire exclusive byte-range lock");

        let mut operations = DeferredOperations::new();
        operations
            .schedule_waiting_lock(
                DeviceIoRequest {
                    device_id: 1,
                    file_id: 7,
                    completion_id: 3,
                    major_function: MajorFunction::LockControl,
                    minor_function: MinorFunction::from(0),
                },
                vec![(0, 8)],
                true,
                waiting_handle,
            )
            .expect("schedule waiting lock");

        let cancelled = operations.cancel_file(1, 7);
        assert_eq!(cancelled.len(), 1);
        let response = cancelled
            .into_iter()
            .next()
            .expect("the cancelled lock has one completion")
            .encode_unframed_pdu()
            .expect("the cancellation completion is encodable");
        assert_eq!(
            u32::from_le_bytes(response[12..16].try_into().expect("response status is present")),
            u32::from(NtStatus::CANCELLED)
        );
        assert!(operations.poll().is_empty());

        let (drive_waiting_handle, _) = root
            .open_relative_file(&relative_path, options)
            .expect("open third temporary file handle");
        operations
            .schedule_waiting_lock(
                DeviceIoRequest {
                    device_id: 1,
                    file_id: 8,
                    completion_id: 4,
                    major_function: MajorFunction::LockControl,
                    minor_function: MinorFunction::from(0),
                },
                vec![(0, 8)],
                true,
                drive_waiting_handle,
            )
            .expect("schedule waiting drive lock");

        let cancelled = operations.cancel_drive(1);
        assert_eq!(cancelled.len(), 1);
        let response = cancelled
            .into_iter()
            .next()
            .expect("the removed drive has one cancellation completion")
            .encode_unframed_pdu()
            .expect("the cancellation completion is encodable");
        assert_eq!(
            u32::from_le_bytes(response[12..16].try_into().expect("response status is present")),
            u32::from(NtStatus::CANCELLED)
        );
        assert!(operations.poll().is_empty());

        locking_handle
            .unlock_range(0, 8)
            .expect("release exclusive byte-range lock");
        std::fs::remove_file(&file_path).expect("remove temporary test file");
    }

    #[test]
    fn closing_a_directory_watch_returns_a_successful_empty_completion() {
        let temporary_directory = std::env::temp_dir().join(format!(
            "ironrdp-rdpdr-deferred-notify-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir(&temporary_directory).expect("create temporary test directory");
        let volume_root = temporary_directory
            .ancestors()
            .last()
            .expect("temporary directory has a volume root");

        {
            let root = RootDirectory::open(volume_root).expect("open temporary volume root");
            let relative_path = temporary_directory
                .strip_prefix(volume_root)
                .expect("temporary directory is beneath its volume root");
            let relative_path = RelativePath::parse(&format!(r"\{}", relative_path.display()))
                .expect("temporary directory has a valid path");
            let directory_handle = root
                .open_relative_directory_for_notification(&relative_path)
                .expect("open asynchronous directory notification handle")
                .expect("existing temporary directory")
                .into_file_handle();

            let mut operations = DeferredOperations::new();
            operations
                .schedule_directory_notification(
                    ServerDriveNotifyChangeDirectoryRequest {
                        device_io_request: DeviceIoRequest {
                            device_id: 1,
                            file_id: 7,
                            completion_id: 3,
                            major_function: MajorFunction::DirectoryControl,
                            minor_function: MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY,
                        },
                        watch_tree: 0,
                        completion_filter: 1,
                    },
                    directory_handle
                        .try_clone()
                        .expect("duplicate directory handle for notification worker"),
                )
                .expect("schedule directory notification");

            let cancelled = operations.cancel_file(1, 7);
            assert_eq!(cancelled.len(), 1);
            let response = cancelled
                .into_iter()
                .next()
                .expect("the cancelled notification has one completion")
                .encode_unframed_pdu()
                .expect("the cancellation completion is encodable");
            assert_eq!(
                u32::from_le_bytes(response[12..16].try_into().expect("response status is present")),
                u32::from(NtStatus::SUCCESS)
            );
            assert_eq!(
                u32::from_le_bytes(response[16..20].try_into().expect("response length is present")),
                0
            );
            assert!(operations.poll().is_empty());
        }

        std::fs::remove_dir_all(&temporary_directory).expect("remove temporary test directory");
    }
}
