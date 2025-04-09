use ironrdp_cliprdr::pdu::ClipboardFormatId;
use tracing::error;
use windows::Win32::Foundation::{GetLastError, GlobalFree, HANDLE, HGLOBAL, WIN32_ERROR};
use windows::Win32::System::DataExchange::SetClipboardData;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

use crate::windows::WinCliprdrResult;

/// Safe wrapper around windows global memory buffer.
struct GlobalMemoryBuffer(HGLOBAL);

impl GlobalMemoryBuffer {
    /// Creates new global memory buffer and copies data into it.
    fn from_slice(data: &[u8]) -> WinCliprdrResult<Self> {
        // SAFETY: GlobalAlloc will return null only if there is not enough memory to allocate
        // `windows` crate will catch this error via internal invalid handle check
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len())? };

        // SAFETY: We created the handle and ensured it wasn’t null just above.
        // Note that we don’t check for failure because we own the handle and
        // know that the specified memory block can’t be discarded at this point.
        let dst = unsafe { GlobalLock(handle) };

        // SAFETY:
        // - `data` is valid for reads of `data.len()` bytes.
        // - `dst` is valid for writes of `data.len()` bytes, we allocated enough above.
        // - Both `data` and `dst` are properly aligned: u8 alignment is 1
        // - Memory regions are not overlapping, `dst` was allocated by us just above.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), dst as *mut u8, data.len()) };

        // SAFETY: We called `GlobalLock` on this handle just above.
        if let Err(error) = unsafe { GlobalUnlock(handle) } {
            error!(%error, "Failed to unlock memory");
        }

        Ok(Self(handle))
    }

    fn as_raw(&self) -> HGLOBAL {
        self.0
    }
}

impl Drop for GlobalMemoryBuffer {
    fn drop(&mut self) {
        // SAFETY: It is safe to call GlobalFree on a valid handle
        if let Err(err) = unsafe { GlobalFree(Some(self.0)) } {
            error!("Failed to free global clipboard data handle: {}", err);
        }
    }
}

/// Render data format into the clipboard.
///
/// SAFETY: This function should only be called in the context of processing
/// `WM_RENDERFORMAT` or `WM_RENDERALLFORMATS` messages inside WinAPI message loop.
pub(crate) unsafe fn render_format(format: ClipboardFormatId, data: &[u8]) -> WinCliprdrResult<()> {
    // Allocate buffer and copy data into it
    let global_data = GlobalMemoryBuffer::from_slice(data)?;

    // Cast HGLOBAL to HANDLE
    let handle = HANDLE(global_data.as_raw().0);

    // SAFETY: If described above safety requirements of `render_format` call are met, then
    // `SetClipboardData` is safe to call.
    let _ = unsafe { SetClipboardData(format.value(), Some(handle)) };

    // We successfully transferred ownership of the data to the clipboard, we don't need to
    // call drop on handle
    core::mem::forget(global_data);

    Ok(())
}

/// Return last WinAPI error code.
pub(crate) fn get_last_winapi_error() -> WIN32_ERROR {
    // SAFETY: `GetLastError` is always safe to call.
    unsafe { GetLastError() }
}
