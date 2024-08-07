use ironrdp_cliprdr::pdu::ClipboardFormatId;
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::DataExchange::GetClipboardData;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

use crate::windows::os_clipboard::OwnedOsClipboard;

/// Wrapper for global clipboard data handle ready for reading.
pub(crate) struct ClipboardDataRef<'a> {
    _os_clipboard: &'a OwnedOsClipboard,
    handle: HGLOBAL,
    data: *const u8,
}

impl<'a> ClipboardDataRef<'a> {
    /// Get new clipboard data from the clipboard. If no data is available for the specified
    /// format, or handle can't be locked, returns `None`.
    pub(crate) fn get(os_clipboard: &'a OwnedOsClipboard, format: ClipboardFormatId) -> Option<Self> {
        // SAFETY: it is safe to call `GetClipboardData`, because we own the clipboard
        // before calling this function.
        let handle = match unsafe { GetClipboardData(format.value()) } {
            Ok(handle) => HGLOBAL(handle.0),
            Err(_) => {
                // No data available for this format
                return None;
            }
        };

        // SAFETY: It is safe to call `GlobalLock` on the valid handle.
        let data = unsafe { GlobalLock(handle) } as *const u8;

        if data.is_null() {
            // Can't lock data handle, handle is not valid anymore (e.g. clipboard has changed)
            return None;
        }

        Some(Self {
            _os_clipboard: os_clipboard,
            handle,
            data,
        })
    }

    /// Returns size of the allocated data in bytes. Note that returned size could be larger than
    /// actual format data size. For format conversion logic, data buffer should be inspected and
    /// its real size should be acquired based on internal format structure.
    ///
    /// E.g. for `CF_TEXT`
    /// format it's required to search for null-terminator to get the actual size of the string.
    pub(crate) fn size(&self) -> usize {
        // SAFETY: We always own non-null handle, so it is safe to call `GlobalSize` on it
        unsafe { GlobalSize(self.handle) }
    }

    /// Pointer to the data buffer available for reading.
    pub(crate) fn data(&self) -> &[u8] {
        let size = self.size();
        // SAFETY: `data` pointer is valid during the lifetime of the wrapper
        unsafe { std::slice::from_raw_parts(self.data, size) }
    }
}

impl Drop for ClipboardDataRef<'_> {
    fn drop(&mut self) {
        // SAFETY: We always own non-null handle, so it is safe to call `GlobalUnlock` on it
        if let Err(err) = unsafe {
            // Handle with data, retrieved from the clipboard, should be unlocked, but not freed
            // (it's owned by the clipboard itself)
            GlobalUnlock(self.handle)
        } {
            tracing::error!("Failed to unlock data: {}", err)
        }
    }
}
