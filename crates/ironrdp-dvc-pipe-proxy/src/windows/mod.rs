//! WinAPI wrappers for the DVC pipe proxy IO loop logic.
//!
//! Some of the wrappers are based on `win-api-wrappers` code (simplified/reduced functionality).

use windows::Win32::Foundation::{
    ERROR_IO_PENDING, HANDLE, WAIT_ABANDONED_0, WAIT_EVENT, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};

mod error;
pub(crate) use self::error::WindowsError;

mod event;
pub(crate) use self::event::Event;

mod pipe;
pub(crate) use self::pipe::MessagePipeServer;

mod semaphore;
pub(crate) use self::semaphore::Semaphore;

/// Thin wrapper around borrowed `windows` crate `HANDLE` reference.
/// This is used to ensure handle lifetime when passing it to FFI functions
/// (see `wait_any_with_timeout` for example).
#[repr(transparent)]
pub(crate) struct BorrowedHandle<'a>(&'a HANDLE);

/// Safe wrapper around `WaitForMultipleObjects`.
pub(crate) fn wait_any_with_timeout(handles: &[BorrowedHandle<'_>], timeout: u32) -> Result<usize, WindowsError> {
    let handles = cast_handles(handles);

    // SAFETY:
    // - BorrowedHandle alongside with rust type system ensures that the HANDLEs are valid for
    // the duration of the call.
    // - All handles in this module have SYNCHRONIZE access rights.
    // - cast_handles ensures no handle duplicates.
    let result = unsafe { WaitForMultipleObjects(handles, false, timeout) };

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

/// Safe `WaitForMultipleObjects` wrapper with infinite timeout.
pub(crate) fn wait_any(handles: &[BorrowedHandle<'_>]) -> Result<usize, WindowsError> {
    // Standard generic syntax is used instead if `impl` because of the following lint:
    // > warning: lifetime parameter `'a` only used once
    //
    // Fixing this lint (use of '_ lifetime) produces compiler error.
    wait_any_with_timeout(handles, INFINITE)
}

fn cast_handles<'a>(handles: &'a [BorrowedHandle<'a>]) -> &'a [HANDLE] {
    // Very basic sanity checks to ensure that the handles are valid
    // and there are no duplicates.
    // This is only done in debug builds to avoid performance overhead in release builds, while
    // still catching undefined behavior early in development.
    #[cfg(debug_assertions)]
    {
        // Ensure that there are no duplicate handles without hash.
        for (i, handle) in handles.iter().enumerate() {
            for other_handle in &handles[i + 1..] {
                if handle.0 == other_handle.0 {
                    panic!("Duplicate handle found in wait_any_with_timeout");
                }
            }
        }
    }

    for handle in handles {
        // Ensure that the handle is valid.
        if handle.0.is_invalid() {
            panic!("Invalid handle in wait_any_with_timeout");
        }
    }

    // SAFETY:
    // - BorrowedHandle is #[repr(transparent)] over *const c_void, and so is HANDLE,
    //   so the layout is the same.
    // - We ensure the lifetime is preserved.
    unsafe { core::slice::from_raw_parts(handles.as_ptr() as *const HANDLE, handles.len()) }
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
