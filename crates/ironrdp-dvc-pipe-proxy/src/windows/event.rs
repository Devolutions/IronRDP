use std::sync::Arc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{CreateEventW, SetEvent};

use crate::windows::{Handle, WindowsError};

/// RAII wrapper for WinAPI event handle.
#[derive(Debug, Clone)]
pub(crate) struct Event {
    handle: Arc<Handle>,
}

/// SAFETY: It is safe to send event HANDLE between threads.
unsafe impl Send for Event {}

impl Event {
    pub(crate) fn new_unnamed() -> Result<Self, WindowsError> {
        // SAFETY: FFI call with no outstanding preconditions.
        let handle = unsafe { CreateEventW(None, false, false, None).map_err(WindowsError::CreateEvent)? };
        // SAFETY: Handle is valid and we are the owner of the handle.
        let handle = unsafe { Handle::new_owned(handle)? };

        // CreateEventW returns a valid handle on success.
        Ok(Self {
            handle: Arc::new(handle),
        })
    }

    pub(crate) fn set(&self) -> Result<(), WindowsError> {
        // SAFETY: The handle is valid and we are the owner of the handle.
        unsafe {
            SetEvent(self.handle.raw()).map_err(WindowsError::SetEvent)?;
        }
        Ok(())
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.handle.raw()
    }
}
