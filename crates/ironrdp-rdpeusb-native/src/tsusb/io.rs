//! Device I/O controls on Windows handles, synchronous or overlapped on an I/O completion port.

use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;
use std::collections::HashMap;

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use tracing::warn;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_IO_PENDING, ERROR_MORE_DATA, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, DeviceIoControl, GetQueuedCompletionStatus, OVERLAPPED,
    PostQueuedCompletionStatus,
};
use windows::Win32::System::Threading::INFINITE;
use windows::core::{Error, HSTRING};

/// How long dropping an [`AsyncDevice`] waits for its cancelled requests to return.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// An owned handle to a device object, or to an I/O completion port.
#[derive(Debug)]
pub(crate) struct DeviceHandle(HANDLE);

// SAFETY: Windows handles are process-wide kernel object references. This wrapper has unique
// ownership and only closes its handle on drop, so moving it between threads cannot create
// concurrent access to anything but the kernel object itself.
unsafe impl Send for DeviceHandle {}
// SAFETY: shared access only reads the immutable handle value; the kernel synchronizes the I/O
// performed on it. The handle is closed once, after every shared reference is released.
unsafe impl Sync for DeviceHandle {}

impl DeviceHandle {
    /// Open a device for synchronous I/O controls, read-only.
    pub(crate) fn open_read(path: &str) -> Result<Self, Error> {
        Self::open(path, GENERIC_READ.0, FILE_SHARE_READ, FILE_ATTRIBUTE_NORMAL)
    }

    /// Open a device for read and write I/O controls, overlapped or synchronous.
    pub(crate) fn open_read_write(path: &str, overlapped: bool) -> Result<Self, Error> {
        let flags = if overlapped {
            FILE_FLAG_OVERLAPPED
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        Self::open(
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            flags,
        )
    }

    fn open(path: &str, access: u32, share: FILE_SHARE_MODE, flags: FILE_FLAGS_AND_ATTRIBUTES) -> Result<Self, Error> {
        let path = HSTRING::from(path);
        // SAFETY: `path` is a NUL-terminated wide string that outlives the call, and every other
        // argument is a plain value or `None`.
        let handle = unsafe { CreateFileW(&path, access, share, None, OPEN_EXISTING, flags, None)? };
        Ok(Self(handle))
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.0
    }

    /// Run a synchronous I/O control, returning the number of output bytes.
    ///
    /// The handle must not have been opened for overlapped I/O.
    pub(crate) fn control(&self, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let mut returned = 0u32;
        self.control_raw(code, input, output, &mut returned)?;
        Ok(usize::try_from(returned).unwrap_or(usize::MAX))
    }

    /// Run a synchronous I/O control whose output size is learnt by a first call without output.
    pub(crate) fn control_vec(&self, code: u32, input: &[u8]) -> Result<Vec<u8>, Error> {
        let mut returned = 0u32;
        match self.control_raw(code, input, &mut [], &mut returned) {
            Ok(()) => return Ok(Vec::new()),
            Err(error)
                if error.code() == ERROR_MORE_DATA.to_hresult()
                    || error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => {}
            Err(error) => return Err(error),
        }

        let mut output = vec![0; usize::try_from(returned).unwrap_or(0)];
        let length = self.control(code, input, &mut output)?;
        output.truncate(length);
        Ok(output)
    }

    fn control_raw(&self, code: u32, input: &[u8], output: &mut [u8], returned: &mut u32) -> Result<(), Error> {
        let input_length = buffer_length(input.len())?;
        let output_length = buffer_length(output.len())?;
        // SAFETY: the buffers are valid for their lengths for the duration of this synchronous call,
        // and empty buffers are passed as null pointers.
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                (!input.is_empty()).then(|| input.as_ptr().cast()),
                input_length,
                (!output.is_empty()).then(|| output.as_mut_ptr().cast()),
                output_length,
                Some(returned),
                None,
            )
        }
    }
}

impl Drop for DeviceHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by CreateFileW or CreateIoCompletionPort and is owned by
        // this guard alone.
        if let Err(error) = unsafe { CloseHandle(self.0) } {
            warn!(%error, "Failed to close a USB redirection handle");
        }
    }
}

fn buffer_length(length: usize) -> Result<u32, Error> {
    u32::try_from(length).map_err(|_| Error::from(ERROR_INSUFFICIENT_BUFFER))
}

/// The outcome of an overlapped I/O control.
#[derive(Debug)]
pub(crate) struct IoResult {
    pub(crate) status: Result<(), Error>,
    /// The input buffer, which some drivers update in place.
    pub(crate) input: Vec<u8>,
    pub(crate) output: Vec<u8>,
    /// Bytes the driver reported as transferred.
    pub(crate) transferred: usize,
}

type Done = Box<dyn FnOnce(IoResult) + Send>;

/// An overlapped request, from its submission until its completion packet is dequeued.
///
/// `overlapped` comes first, so that the `OVERLAPPED` pointer returned by the completion port is
/// also a pointer to the request.
#[repr(C)]
struct Pending {
    overlapped: OVERLAPPED,
    key: u64,
    input: Vec<u8>,
    output: Vec<u8>,
    done: Done,
}

/// A request owned by the kernel while it runs.
struct InFlight(NonNull<Pending>);

// SAFETY: the pointed-to request is only dereferenced by the completion thread, after the kernel
// returned it and it was removed from the in-flight map; elsewhere only its address is used, to cancel
// it.
unsafe impl Send for InFlight {}

struct Shared {
    device: DeviceHandle,
    port: DeviceHandle,
    in_flight: Mutex<HashMap<u64, InFlight>>,
    /// Signalled whenever the last request in flight completes.
    idle: Condvar,
    next_key: AtomicU64,
}

impl Shared {
    fn in_flight(&self) -> MutexGuard<'_, HashMap<u64, InFlight>> {
        // Every update is a single map operation, so the map stays consistent across a panic.
        self.in_flight.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Take a request out of the in-flight map, returning whether it was there, and wake whoever
    /// waits for the last request to return.
    fn take_in_flight(&self, key: u64) -> bool {
        let mut in_flight = self.in_flight();
        let removed = in_flight.remove(&key).is_some();
        if in_flight.is_empty() {
            self.idle.notify_all();
        }
        removed
    }
}

/// A device opened for overlapped I/O controls, completed on a dedicated thread.
pub(crate) struct AsyncDevice {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
}

impl AsyncDevice {
    /// Bind an overlapped device handle to a new completion port and start its completion thread.
    pub(crate) fn new(device: DeviceHandle, name: &str) -> Result<Self, Error> {
        // SAFETY: `device` is a valid handle opened for overlapped I/O; a new port is created.
        let port = unsafe { CreateIoCompletionPort(device.raw(), None, 0, 1)? };
        let shared = Arc::new(Shared {
            device,
            port: DeviceHandle(port),
            in_flight: Mutex::new(HashMap::new()),
            idle: Condvar::new(),
            next_key: AtomicU64::new(0),
        });

        let thread = thread::Builder::new().name(name.to_owned()).spawn({
            let shared = Arc::clone(&shared);
            move || run_completions(&shared)
        })?;

        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// A key for the next request, unique for this device.
    pub(crate) fn next_key(&self) -> u64 {
        self.shared.next_key.fetch_add(1, Ordering::Relaxed)
    }

    /// Submit an overlapped I/O control under `key`, which cancels it.
    ///
    /// `done` runs on the completion thread once the request completes, or right away when the
    /// driver rejects it.
    pub(crate) fn submit(
        &self,
        key: u64,
        code: u32,
        input: Vec<u8>,
        output: Vec<u8>,
        done: impl FnOnce(IoResult) + Send + 'static,
    ) {
        let (Ok(input_length), Ok(output_length)) = (buffer_length(input.len()), buffer_length(output.len())) else {
            done(IoResult {
                status: Err(Error::from(ERROR_INSUFFICIENT_BUFFER)),
                input,
                output,
                transferred: 0,
            });
            return;
        };

        let mut pending = Box::new(Pending {
            overlapped: OVERLAPPED::default(),
            key,
            input,
            output,
            done: Box::new(done),
        });
        // The buffers are separate allocations, which leaking the request does not move.
        let input_buffer = (!pending.input.is_empty()).then(|| pending.input.as_ptr().cast());
        let output_buffer = (!pending.output.is_empty()).then(|| pending.output.as_mut_ptr().cast());
        let pending = NonNull::from(Box::leak(pending));
        // Registered first: the completion thread may dequeue the request before the call returns.
        self.shared.in_flight().insert(key, InFlight(pending));

        let raw = pending.as_ptr();
        // SAFETY: the request is leaked above and only freed by the completion thread, or below when
        // the driver rejects it: its buffers and its `OVERLAPPED`, the first field of a `repr(C)`
        // struct, stay valid and unmoved until the kernel completes it. Empty buffers are null.
        let result = unsafe {
            DeviceIoControl(
                self.shared.device.raw(),
                code,
                input_buffer,
                input_length,
                output_buffer,
                output_length,
                None,
                Some(raw.cast::<OVERLAPPED>()),
            )
        };

        match result {
            // Success and pending both queue a completion packet.
            Ok(()) => {}
            Err(error) if error.code() == ERROR_IO_PENDING.to_hresult() => {}
            Err(error) => {
                // A request rejected right away queues no completion packet: complete it here.
                if self.shared.take_in_flight(key) {
                    // SAFETY: the request was registered above and never reached the kernel, so this is
                    // the only reference left to it.
                    let pending = unsafe { Box::from_raw(raw) };
                    let Pending {
                        input, output, done, ..
                    } = *pending;
                    done(IoResult {
                        status: Err(error),
                        input,
                        output,
                        transferred: 0,
                    });
                }
            }
        }
    }

    /// Cancel a request, which then completes with an error unless it already finished.
    ///
    /// `cancel` receives the device handle and the address of the request's `OVERLAPPED`, for drivers
    /// with their own cancellation, and returns whether it cancelled the request; otherwise the I/O is
    /// cancelled with `CancelIoEx`. The request cannot complete meanwhile.
    pub(crate) fn cancel_with(&self, key: u64, cancel: impl FnOnce(HANDLE, usize) -> bool) {
        let in_flight = self.shared.in_flight();
        if let Some(request) = in_flight.get(&key) {
            // The lock is held, so the completion thread cannot free the request meanwhile.
            let overlapped = request.0.as_ptr().cast::<OVERLAPPED>();
            if !cancel(self.shared.device.raw(), overlapped.addr()) {
                // SAFETY: `overlapped` belongs to a request the kernel still owns; cancelling an I/O
                // that just finished only fails with ERROR_NOT_FOUND.
                let _ = unsafe { CancelIoEx(self.shared.device.raw(), Some(overlapped.cast_const())) };
            }
        }
    }
}

impl Drop for AsyncDevice {
    fn drop(&mut self) {
        // SAFETY: cancelling every I/O of a valid handle has no other effect.
        let _ = unsafe { CancelIoEx(self.shared.device.raw(), None) };

        let in_flight = self.shared.in_flight();
        let (in_flight, timeout) = self
            .shared
            .idle
            .wait_timeout_while(in_flight, SHUTDOWN_TIMEOUT, |in_flight| !in_flight.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        if timeout.timed_out() {
            // The requests stay leaked: the kernel may still write to them.
            warn!(
                count = in_flight.len(),
                "USB redirection requests did not return after being cancelled"
            );
        }
        drop(in_flight);

        // A packet without an `OVERLAPPED` stops the completion thread.
        // SAFETY: the port is a valid completion port owned by `shared`.
        if unsafe { PostQueuedCompletionStatus(self.shared.port.raw(), 0, 0, None) }.is_ok()
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

fn run_completions(shared: &Shared) {
    loop {
        let mut transferred = 0u32;
        let mut completion_key = 0usize;
        let mut overlapped: *mut OVERLAPPED = ptr::null_mut();
        // SAFETY: the out-pointers are valid locals and the port stays open while `shared` lives.
        let status = unsafe {
            GetQueuedCompletionStatus(
                shared.port.raw(),
                &mut transferred,
                &mut completion_key,
                &mut overlapped,
                INFINITE,
            )
        };
        let Some(overlapped) = NonNull::new(overlapped) else {
            // The stop packet, or the port is gone.
            return;
        };

        let raw = overlapped.cast::<Pending>().as_ptr();
        // SAFETY: every `OVERLAPPED` queued on this port is the first field of a `Pending` leaked by
        // `submit`, which the kernel returned along with ownership of it.
        let key = unsafe { (*raw).key };
        if !shared.take_in_flight(key) {
            continue;
        }

        // SAFETY: the request was removed from the in-flight map under its lock, so nothing else
        // refers to it, and it was allocated by `Box::new` in `submit`.
        let pending = unsafe { Box::from_raw(raw) };
        let Pending {
            input, output, done, ..
        } = *pending;
        done(IoResult {
            status,
            input,
            output,
            transferred: usize::try_from(transferred).unwrap_or(usize::MAX),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_devices_fail_to_open() {
        let error = DeviceHandle::open_read(r"\\.\IronRdpNoSuchUsbRedirectionDevice").expect_err("no such device");
        assert!(error.code().is_err());
    }
}
