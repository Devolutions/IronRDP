use windows::Win32::Foundation::{CloseHandle, HANDLE};

use crate::windows::WindowsError;

/// A wrapper around a Windows [`HANDLE`].
///
/// Whenever possible, you should use [`BorrowedHandle`] or [`OwnedHandle`] instead.
/// Those are safer to use.
#[derive(Debug, Clone)]
pub(crate) struct Handle {
    raw: HANDLE,
}

// SAFETY: A `HANDLE` is, by definition, thread safe.
unsafe impl Send for Handle {}

// SAFETY: A `HANDLE` is simply an integer, no dereferencing is done.
unsafe impl Sync for Handle {}

/// The `Drop` implementation is assuming we constructed the `Handle` object in
/// a sane way to call `CloseHandle`, but there is no way for us to verify that
/// the handle is actually owned outside of the callsite. Conceptually, calling
/// `Handle::new_owned(handle)` is like calling the unsafe function `CloseHandle`
/// and thus must inherit its safety preconditions.
impl Handle {
    /// Wraps an owned Windows [`HANDLE`].
    ///
    /// # Safety
    ///
    /// - `handle` is a valid handle to an open object.
    /// - `handle` is not a pseudohandle.
    /// - The caller is actually responsible for closing the `HANDLE` when
    ///   the value goes out of scope.
    pub(crate) unsafe fn new_owned(handle: HANDLE) -> Result<Self, WindowsError> {
        if handle.is_invalid() || handle.0.is_null() {
            return Err(WindowsError::InvalidHandle);
        }

        // SAFETY: Same preconditions as the called function.
        Ok(Self { raw: handle })
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.raw
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: `self.raw` is a valid handle to an open object by construction.
        let _ = unsafe { CloseHandle(self.raw) };
    }
}
