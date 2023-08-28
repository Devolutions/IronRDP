use crate::backend::windows::{WinCliprdrError, WinCliprdrResult};
use crate::pdu::ClipboardFormatId;

use winapi::shared::{minwindef::HGLOBAL, winerror::ERROR_SUCCESS};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::winbase::{GlobalAlloc, GlobalFree, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use winapi::um::winuser::SetClipboardData;

/// Safe wrapper around windows global clipboard data handle.
pub struct ClipboardData(HGLOBAL);

impl ClipboardData {
    /// Renders data into the clipboard. Should be only invoked in the context of processing message
    /// `WM_RENDERFORMAT` or `WM_RENDERALLFORMATS`.
    pub fn render(format: ClipboardFormatId, data: &[u8]) -> WinCliprdrResult<()> {
        // Allocate buffer and copy data into it
        let global_data = Self::from_slice(data)?;

        if unsafe { SetClipboardData(format.value(), global_data.handle()) }.is_null()
            && unsafe { GetLastError() } != ERROR_SUCCESS
        {
            return Err(WinCliprdrError::RenderFormat);
        }

        // We successfully transfered ownership of the data to the clipboard, we don't need to
        // call drop on it
        std::mem::forget(global_data);

        Ok(())
    }

    /// Creates new global buffer and copies data into it.
    fn from_slice(data: &[u8]) -> WinCliprdrResult<Self> {
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len()) };

        if handle.is_null() {
            return Err(WinCliprdrError::Alloc);
        }

        // We own the handle, so it is safe to assume that GlobalLock will succeed
        unsafe {
            let dst = GlobalLock(handle);
            std::ptr::copy(data.as_ptr(), dst as _, data.len());
            GlobalUnlock(handle);
        };

        Ok(Self(handle))
    }

    fn handle(&self) -> HGLOBAL {
        self.0
    }
}

impl Drop for ClipboardData {
    fn drop(&mut self) {
        unsafe { GlobalFree(self.0) };
    }
}
