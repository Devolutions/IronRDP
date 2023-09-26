use ironrdp_cliprdr::pdu::ClipboardFormatId;
use tracing::error;
use windows::Win32::Foundation::{GetLastError, HANDLE, HGLOBAL, WIN32_ERROR};
use windows::Win32::System::DataExchange::SetClipboardData;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalFree, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

use crate::windows::WinCliprdrResult;

/// Safe wrapper around windows global memory buffer.
struct GlobalMemoryBuffer(HGLOBAL);

impl GlobalMemoryBuffer {
    /// Creates new global memory buffer and copies data into it.
    fn from_slice(data: &[u8]) -> WinCliprdrResult<Self> {
        // SAFETY: GlobalAlloc will return null only if there is not enough memory to allocate
        // `windows` crate will catch this error via internal invalid handle check
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len())? };

        // SAFETY: We own the handle, so it is safe to assume that GlobalLock will succeed
        unsafe {
            let dst = GlobalLock(handle);
            std::ptr::copy(data.as_ptr(), dst as _, data.len());
            GlobalUnlock(handle);
        };

        Ok(Self(handle))
    }

    fn as_raw(&self) -> HGLOBAL {
        self.0
    }
}

impl Drop for GlobalMemoryBuffer {
    fn drop(&mut self) {
        // SAFETY: It is safe to call GlobalFree on a valid handle
        if let Err(err) = unsafe { GlobalFree(self.0) } {
            error!("Failed to free global clipboard data handle: {}", err);
        }
    }
}

/// Render data format into the clipboard.
///
/// SAFETY: This function should only be called in the context of processing
/// `WM_RENDERFORMAT` or `WM_RENDERALLFORMATS` messages inside WinAPI message loop.
pub unsafe fn render_format(format: ClipboardFormatId, data: &[u8]) -> WinCliprdrResult<()> {
    // Allocate buffer and copy data into it
    let global_data = GlobalMemoryBuffer::from_slice(data)?;

    // Cast HGLOBAL to HANDLE
    let handle = HANDLE(global_data.as_raw().0);

    let _ = SetClipboardData(format.value(), handle);

    // We successfully transferred ownership of the data to the clipboard, we don't need to
    // call drop on handle
    std::mem::forget(global_data);

    Ok(())
}

/// Return last WinAPI error code.
pub fn get_last_winapi_error() -> WIN32_ERROR {
    // SAFETY: `GetLastError` is always safe to call.
    unsafe { GetLastError() }
}
