use core::sync::atomic::{AtomicBool, Ordering};
use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::sync::Arc;

use anyhow::Context as _;
use tokio::task::JoinHandle;
use windows::Win32::Foundation::{
    ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
    PeekNamedPipe,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::{PCWSTR, w};

use crate::windows_main::security_attributes;

pub(crate) struct NativePipe(Arc<PipeHandle>);

struct PipeHandle {
    handle: OwnedHandle,
    cancelled: AtomicBool,
}

impl NativePipe {
    pub(crate) fn create(name: &str) -> anyhow::Result<Self> {
        Self::create_with_access(name, w!("D:P(A;;GA;;;SY)(A;;GA;;;NS)"))
    }

    fn create_with_access(name: &str, access: PCWSTR) -> anyhow::Result<Self> {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let (attributes, descriptor) = security_attributes(access)?;
        // SAFETY: Both the NUL-terminated name and security descriptor remain live throughout this call.
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                64 * 1024,
                64 * 1024,
                0,
                Some(&attributes),
            )
        };
        let error = handle.is_invalid().then(io::Error::last_os_error);
        // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocated this descriptor.
        let _ = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        if let Some(error) = error {
            return Err(error).context("create native message pipe");
        }
        // SAFETY: CreateNamedPipe returned a valid handle whose ownership has not been transferred.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        Ok(Self(Arc::new(PipeHandle {
            handle,
            cancelled: AtomicBool::new(false),
        })))
    }

    pub(crate) async fn connect(&self) -> anyhow::Result<()> {
        let pipe = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || pipe.perform(Operation::Connect))
            .await
            .context("native pipe connection worker failed")??;
        Ok(())
    }

    pub(crate) fn read(&self, maximum: usize) -> JoinHandle<anyhow::Result<Vec<u8>>> {
        let pipe = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || {
            let mut buffer = vec![0; maximum];
            let length = pipe
                .perform(Operation::Read(&mut buffer))
                .context("read native pipe message")?;
            buffer.truncate(usize::try_from(length)?);
            if length == 0 {
                // Empty messages are valid; only a broken pipe is EOF.
                // SAFETY: This worker retains ownership of the pipe handle.
                unsafe { PeekNamedPipe(HANDLE(pipe.handle.as_raw_handle()), None, 0, None, None, None) }?;
            }
            Ok(buffer)
        })
    }

    pub(crate) async fn write(&self, record: Vec<u8>) -> anyhow::Result<()> {
        let pipe = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || {
            let length = pipe
                .perform(Operation::Write(&record))
                .context("write native pipe message")?;
            anyhow::ensure!(usize::try_from(length)? == record.len(), "partial native pipe write");
            anyhow::Ok(())
        })
        .await
        .context("native pipe write worker failed")?
    }

    #[cfg(test)]
    pub(crate) fn for_test(name: &str) -> anyhow::Result<Self> {
        // The production pipe admits service identities; loopback tests use their unprivileged owner.
        Self::create_with_access(name, w!("D:P(A;;GA;;;OW)"))
    }
}

impl Drop for NativePipe {
    fn drop(&mut self) {
        self.0.cancelled.store(true, Ordering::Release);
        // SAFETY: Workers retain the handle and their buffers until cancelled operations finish.
        let _ = unsafe { CancelIoEx(HANDLE(self.0.handle.as_raw_handle()), None) };
    }
}

enum Operation<'a> {
    Connect,
    Read(&'a mut [u8]),
    Write(&'a [u8]),
}

impl PipeHandle {
    fn perform(&self, operation: Operation<'_>) -> anyhow::Result<u32> {
        anyhow::ensure!(!self.cancelled.load(Ordering::Acquire), "native pipe was cancelled");
        // SAFETY: No pointers are supplied; the returned event belongs to this operation.
        let event = unsafe { CreateEventW(None, true, false, None) }?;
        // SAFETY: The new event handle has a single owner.
        let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
        let mut overlapped = OVERLAPPED {
            hEvent: HANDLE(event.as_raw_handle()),
            ..Default::default()
        };
        let handle = HANDLE(self.handle.as_raw_handle());
        let deadline = match operation {
            Operation::Read(_) => INFINITE,
            Operation::Connect => 3000,
            Operation::Write(_) => 10_000,
        };
        let result = match operation {
            // SAFETY: OVERLAPPED stays live and stationary until completion below.
            Operation::Connect => unsafe { ConnectNamedPipe(handle, Some(&mut overlapped)) },
            // SAFETY: The output buffer and OVERLAPPED stay live and stationary until completion below.
            Operation::Read(buffer) => unsafe { ReadFile(handle, Some(buffer), None, Some(&mut overlapped)) },
            // SAFETY: The input buffer and OVERLAPPED stay live and stationary until completion below.
            Operation::Write(buffer) => unsafe { WriteFile(handle, Some(buffer), None, Some(&mut overlapped)) },
        };
        if let Err(error) = result {
            if error.code() == ERROR_PIPE_CONNECTED.to_hresult() {
                return Ok(0);
            }
            if error.code() != ERROR_IO_PENDING.to_hresult() {
                return Err(error.into());
            }
        }
        if self.cancelled.load(Ordering::Acquire) {
            // SAFETY: This exact OVERLAPPED belongs to the submitted operation.
            let _ = unsafe { CancelIoEx(handle, Some(&overlapped)) };
        }
        // SAFETY: This worker owns the event until the pending operation completes.
        let wait = unsafe { WaitForSingleObject(HANDLE(event.as_raw_handle()), deadline) };
        if wait != WAIT_OBJECT_0 {
            // SAFETY: Cancellation does not free the operation; GetOverlappedResult below joins it.
            let _ = unsafe { CancelIoEx(handle, Some(&overlapped)) };
        }
        let mut length = 0;
        // SAFETY: Wait even after cancellation so no buffer or OVERLAPPED is freed during kernel I/O.
        let completed = unsafe { GetOverlappedResult(handle, &overlapped, &mut length, true) };
        anyhow::ensure!(wait != WAIT_TIMEOUT, "native pipe operation timed out");
        anyhow::ensure!(wait == WAIT_OBJECT_0, "native pipe event wait failed");
        completed?;
        anyhow::ensure!(!self.cancelled.load(Ordering::Acquire), "native pipe was cancelled");
        Ok(length)
    }
}
