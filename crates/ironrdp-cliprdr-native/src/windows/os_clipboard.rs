use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt as _;
use std::path::PathBuf;

use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, ClipboardFormatName};
use tracing::error;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardFormatNameW, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use crate::windows::WinCliprdrError;
use crate::windows::utils::get_last_winapi_error;

/// Safe wrapper around windows. Clipboard is automatically closed on drop.
pub(crate) struct OwnedOsClipboard;

impl OwnedOsClipboard {
    pub(crate) fn new(window: HWND) -> Result<Self, WinCliprdrError> {
        // SAFETY: `window` is valid handle, therefore it is safe to call `OpenClipboard`.
        unsafe { OpenClipboard(Some(window))? };
        Ok(Self)
    }

    /// Enumerates all available formats in the current clipboard.
    #[expect(clippy::unused_self)] // ensure we own the clipboard using RAII, and exclusive &mut self reference
    pub(crate) fn enum_available_formats(&mut self) -> Result<Vec<ClipboardFormat>, WinCliprdrError> {
        const DEFAULT_FORMATS_CAPACITY: usize = 16;
        // Sane default for format name. If format name is longer than this,
        // `GetClipboardFormatNameW` will truncate it.
        const MAX_FORMAT_NAME_LENGTH: usize = 256;

        let mut formats = Vec::with_capacity(DEFAULT_FORMATS_CAPACITY);

        // SAFETY: We own the clipboard at moment of method invocation, therefore it is safe to
        // call `EnumClipboardFormats`.
        let mut raw_format = unsafe { EnumClipboardFormats(0) };
        let mut format_name_w = [0u16; MAX_FORMAT_NAME_LENGTH];

        while raw_format != 0 {
            let format_id = ClipboardFormatId::new(raw_format);

            // Get format name for predefined formats
            let format = if !format_id.is_standard() {
                // SAFETY: It is safe to call `GetClipboardFormatNameW` with correct buffer pointer
                // and size (wrapped as slice via `windows` crate)
                let read_chars: usize = unsafe { GetClipboardFormatNameW(raw_format, &mut format_name_w) }
                    .try_into()
                    .expect("never negative");

                if read_chars != 0 {
                    let format_name = String::from_utf16(format_name_w[..read_chars].as_ref())
                        .map_err(|_| WinCliprdrError::Utf16Conversion)?;

                    ClipboardFormat::new(format_id).with_name(ClipboardFormatName::new(format_name))
                } else {
                    // Unknown format without explicit name
                    ClipboardFormat::new(format_id)
                }
            } else {
                ClipboardFormat::new(format_id)
            };

            formats.push(format);

            // SAFETY: Same as above, we own the clipboard at moment of method invocation, therefore
            // it is safe to call `EnumClipboardFormats`.
            raw_format = unsafe { EnumClipboardFormats(raw_format) };
        }

        if get_last_winapi_error().is_err() {
            return Err(WinCliprdrError::FormatsEnumeration);
        }

        Ok(formats)
    }

    /// Returns the paths in `CF_HDROP`, or `None` if the clipboard has no file drop.
    #[expect(clippy::unused_self)] // ensure we own the clipboard using RAII, and exclusive &mut self reference
    pub(crate) fn dropped_files(&mut self) -> Option<Vec<PathBuf>> {
        // SAFETY: We own the clipboard at moment of method invocation, therefore it is safe to
        // call `GetClipboardData`.
        let handle = unsafe { GetClipboardData(ClipboardFormatId::CF_HDROP.value()) }.ok()?;
        let hdrop = HDROP(handle.0);

        // SAFETY: `hdrop` is the clipboard's CF_HDROP handle, valid while the clipboard is open;
        // index 0xFFFFFFFF queries the file count.
        let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };

        let mut paths = Vec::new();
        for index in 0..count {
            // SAFETY: Same as above; a missing buffer queries the path length.
            let len = unsafe { DragQueryFileW(hdrop, index, None) };
            let mut buffer = vec![0u16; usize::try_from(len).ok()?.saturating_add(1)];
            // SAFETY: Same as above; `buffer` has room for the path and its null terminator.
            let copied = unsafe { DragQueryFileW(hdrop, index, Some(&mut buffer)) };
            buffer.truncate(usize::try_from(copied).ok()?);
            paths.push(PathBuf::from(OsString::from_wide(&buffer)));
        }

        Some(paths)
    }

    /// Empties the clipboard
    ///
    /// It is required to empty clipboard before setting any delay-rendered data.
    #[expect(clippy::unused_self)] // ensure we own the clipboard using RAII, and exclusive &mut self reference
    pub(crate) fn clear(&mut self) -> Result<(), WinCliprdrError> {
        // SAFETY: We own the clipboard at moment of method invocation, therefore it is safe to
        // call `EmptyClipboard`.
        unsafe { EmptyClipboard()? };

        Ok(())
    }

    #[expect(clippy::unused_self)] // ensure we own the clipboard using RAII, and exclusive &mut self reference
    pub(crate) fn delay_render(&mut self, format: ClipboardFormatId) -> Result<(), WinCliprdrError> {
        // SAFETY: We own the clipboard at moment of method invocation, therefore it is safe to
        // call `SetClipboardData`.
        let result = unsafe { SetClipboardData(format.value(), None) };

        if let Err(err) = result {
            // `windows` crate will return `Err(..)` on err zero handle, but for `SetClipboardData`
            // is is considered as error only if `GetLastError` returns non-zero value
            if err.code().is_err() {
                error!("Failed to delayed clipboard rendering for format {}", format.value());
                return Err(WinCliprdrError::SetClipboardData);
            }
        }

        Ok(())
    }
}

impl Drop for OwnedOsClipboard {
    fn drop(&mut self) {
        // SAFETY: We own the clipboard at moment of method invocation, therefore it is safe to
        // call `CloseClipboard`.
        if let Err(err) = unsafe { CloseClipboard() } {
            error!("Failed to close clipboard: {}", err);
        }
    }
}
