//! WinAPI wrappers for the DVC pipe proxy IO loop logic.
//!
//! Some of the wrappers are based on `win-api-wrappers` code (simplified/reduced functionality).

mod error;
mod event;
mod pipe;
mod semaphore;
mod wide_string;

pub(crate) use error::WindowsError;
pub(crate) use event::Event;
pub(crate) use pipe::MessagePipeServer;
pub(crate) use semaphore::Semaphore;
pub(crate) use wide_string::WideString;
use windows::Win32::Foundation::{
    ERROR_IO_PENDING, HANDLE, WAIT_ABANDONED_0, WAIT_EVENT, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};

// WaitForMultipleObjects wrapper with timeout.
pub(crate) fn wait_any_with_timeout(handles: &[HANDLE], timeout: u32) -> Result<usize, WindowsError> {
    // SAFETY: FFI call with no outstanding preconditions.
    let result = unsafe {
        WaitForMultipleObjects(handles, false, timeout)
    };

    match result {
        WAIT_FAILED => Err(WindowsError::WaitForMultipleObjectsFailed(
            windows::core::Error::from_win32(),
        )),
        WAIT_TIMEOUT => Err(WindowsError::WaitForMultipleObjectsTimeout),
        WAIT_EVENT(idx) if idx >= WAIT_ABANDONED_0.0 => {
            let idx = idx - WAIT_ABANDONED_0.0;
            Err(WindowsError::WaitForMultipleObjectsAbandoned(idx))
        }
        WAIT_EVENT(id) => Ok((id - WAIT_OBJECT_0.0) as usize),
    }
}

/// WaitForMultipleObjects wrapper with infinite timeout.
pub(crate) fn wait_any(events: &[HANDLE]) -> Result<usize, WindowsError> {
    wait_any_with_timeout(events, INFINITE)
}

/// Maps ERROR_IO_PENDING to Ok(()) and returns other errors as is.
fn ensure_overlapped_io_result(result: windows::core::Result<()>) -> Result<windows::core::Result<()>, WindowsError> {
    if let Err(error) = &result {
        if error.code() == ERROR_IO_PENDING.to_hresult() {
            return Ok(Ok(()));
        }
    }

    Ok(result)
}
