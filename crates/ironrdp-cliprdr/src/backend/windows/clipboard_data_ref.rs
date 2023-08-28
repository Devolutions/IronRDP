use crate::backend::windows::os_clipboard::OwnedOsClipboard;
use crate::pdu::ClipboardFormatId;

use winapi::shared::minwindef::HGLOBAL;
use winapi::um::winbase::{GlobalLock, GlobalSize, GlobalUnlock};
use winapi::um::winuser::GetClipboardData;

/// Wrapper for global clipboard data handle ready for reading.
pub struct ClipboardDataRef<'a> {
    _os_clipboard: &'a OwnedOsClipboard,
    handle: HGLOBAL,
}

impl<'a> ClipboardDataRef<'a> {
    /// Get new clipboard data from the clipboard. If no data is available for the specified
    /// format, or handle can't be locked, returns `None`.
    pub fn get(os_clipboard: &'a OwnedOsClipboard, format: ClipboardFormatId) -> Option<Self> {
        let handle = unsafe { GetClipboardData(format.value()) };

        if handle.is_null() {
            // No data available for this format
            return None;
        }

        let data = unsafe { GlobalLock(handle) };

        if data.is_null() {
            // Can't lock data handle, handle is not valid anymore (e.g. clipboard has changed)
            return None;
        }

        Some(Self {
            _os_clipboard: os_clipboard,
            handle,
        })
    }

    /// Returns size of the allocated data in bytes. Note that returned size could be larger than
    /// actual format data size. For format conversion logic, data buffer should be inspected and
    /// its real size should be acquired based on internal format structure.
    ///
    /// E.g. for `CF_TEXT`
    /// format it's required to search for null-terminator to get the actual size of the string.
    pub fn size(&self) -> usize {
        unsafe { GlobalSize(self.handle) }
    }

    /// Pointer to the data buffer available for reading.
    pub fn data(&self) -> &[u8] {
        let size = self.size();
        unsafe { std::slice::from_raw_parts(self.handle as *const u8, size) }
    }
}

impl Drop for ClipboardDataRef<'_> {
    fn drop(&mut self) {
        unsafe {
            // Handle with data, retrieved from the clipboard, should be unlocked, but not freed
            // (it's owned by the clipboard itself)
            GlobalUnlock(self.handle);
        }
    }
}
