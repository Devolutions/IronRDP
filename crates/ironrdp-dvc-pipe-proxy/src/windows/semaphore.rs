use std::sync::Arc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{CreateSemaphoreW, ReleaseSemaphore};

use crate::windows::{Handle, WindowsError};

/// RAII wrapper for WinAPI semaphore handle.
#[derive(Debug, Clone)]
pub(crate) struct Semaphore {
    handle: Arc<Handle>,
}

/// SAFETY: It is safe to send semaphore HANDLE between threads.
unsafe impl Send for Semaphore {}

impl Semaphore {
    /// Creates a new unnamed semaphore with the specified initial and maximum counts.
    pub(crate) fn new_unnamed(initial_count: u32, maximum_count: u32) -> Result<Self, WindowsError> {
        if maximum_count == 0 {
            return Err(WindowsError::InvalidSemaphoreParams(
                "maximum_count must be greater than 0",
            ));
        }

        if initial_count > maximum_count {
            return Err(WindowsError::InvalidSemaphoreParams(
                "initial_count must be less than or equal to maximum_count",
            ));
        }

        let initial_count = i32::try_from(initial_count)
            .map_err(|_| WindowsError::InvalidSemaphoreParams("initial_count should be positive"))?;

        let maximum_count = i32::try_from(maximum_count)
            .map_err(|_| WindowsError::InvalidSemaphoreParams("maximum_count should be positive"))?;

        // SAFETY: All parameters are checked for validity above:
        // - initial_count is always <= maximum_count.
        // - maximum_count is always > 0.
        // - all values are positive.
        let handle = unsafe {
            CreateSemaphoreW(None, initial_count, maximum_count, None).map_err(WindowsError::CreateSemaphore)?
        };

        // SAFETY: Handle is valid and we are the owner of the handle.
        let handle = unsafe { Handle::new_owned(handle)? };

        // CreateSemaphoreW returns a valid handle on success.
        Ok(Self {
            handle: Arc::new(handle),
        })
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.handle.raw()
    }

    pub(crate) fn release(&self, release_count: u16) -> Result<u32, WindowsError> {
        let release_count = i32::from(release_count);

        if release_count == 0 {
            // semaphore release count must be greater than 0
            return Err(WindowsError::InvalidSemaphoreParams(
                "release_count must be greater than 0",
            ));
        }

        let mut previous_count = 0;
        // SAFETY: All parameters are checked for validity above:
        // - release_count > 0.
        // - lpPreviousCount points to valid stack memory.
        // - handle is valid and owned by this struct.
        unsafe {
            ReleaseSemaphore(self.handle.raw(), release_count, Some(&mut previous_count))
                .map_err(WindowsError::ReleaseSemaphore)?;
        }
        Ok(previous_count.try_into().expect("semaphore count is negative"))
    }
}
