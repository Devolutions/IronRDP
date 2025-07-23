use std::sync::Arc;

use windows::core::Owned;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{CreateEventW, SetEvent};

use crate::windows::{BorrowedHandle, WindowsError};

/// RAII wrapper for WinAPI event handle.
#[derive(Debug, Clone)]
pub(crate) struct Event {
    handle: Arc<Owned<HANDLE>>,
}

// SAFETY: We ensure that inner handle is indeed could be sent and shared between threads via
// Event wrapper API itself by restricting handle usage:
// - set() method which calls SetEvent inside (which is thread-safe).
// - borrow() method which returns a BorrowedHandle for waiting on the event.
// - Handle lifetime is ensured by Arc, so it is always valid when used.
unsafe impl Send for Event {}

impl Event {
    pub(crate) fn new_unnamed() -> Result<Self, WindowsError> {
        // SAFETY: FFI call with no outstanding preconditions.
        let handle = unsafe { CreateEventW(None, false, false, None).map_err(WindowsError::CreateEvent)? };

        // SAFETY: Handle is valid and we are the owner of the handle.
        let handle = unsafe { Owned::new(handle) };

        // CreateEventW returns a valid handle on success.
        Ok(Self {
            // See `unsafe impl Send` comment.
            #[expect(clippy::arc_with_non_send_sync)]
            handle: Arc::new(handle),
        })
    }

    pub(crate) fn set(&self) -> Result<(), WindowsError> {
        // SAFETY: The handle is valid and we are the owner of the handle.
        unsafe {
            SetEvent(self.raw()).map_err(WindowsError::SetEvent)?;
        }
        Ok(())
    }

    pub(super) fn raw(&self) -> HANDLE {
        **self.handle
    }

    pub(crate) fn borrow(&self) -> BorrowedHandle<'_> {
        BorrowedHandle(&self.handle)
    }
}
