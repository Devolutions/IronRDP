use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, ClipboardFormatName};
use tracing::error;
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardFormatNameW, OpenClipboard, SetClipboardData,
};

use crate::windows::utils::get_last_winapi_error;
use crate::windows::WinCliprdrError;

/// Safe wrapper around windows. Clipboard is automatically closed on drop.
pub(crate) struct OwnedOsClipboard;

impl OwnedOsClipboard {
    pub(crate) fn new(window: HWND) -> Result<Self, WinCliprdrError> {
        // SAFETY: `window` is valid handle, therefore it is safe to call `OpenClipboard`.
        unsafe { OpenClipboard(window)? };
        Ok(Self)
    }

    /// Enumerates all available formats in the current clipboard.
    #[allow(clippy::unused_self)] // ensure we own the clipboard using RAII, and exclusive &mut self reference
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
                        .map_err(|_| WinCliprdrError::Uft16Conversion)?;

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

    /// Empties the clipboard
    ///
    /// It is required to empty clipboard before setting any delay-rendered data.
    #[allow(clippy::unused_self)] // ensure we own the clipboard using RAII, and exclusive &mut self reference
    pub(crate) fn clear(&mut self) -> Result<(), WinCliprdrError> {
        // SAFETY: We own the clipboard at moment of method invocation, therefore it is safe to
        // call `EmptyClipboard`.
        unsafe { EmptyClipboard()? };

        Ok(())
    }

    #[allow(clippy::unused_self)] // ensure we own the clipboard using RAII, and exclusive &mut self reference
    pub(crate) fn delay_render(&mut self, format: ClipboardFormatId) -> Result<(), WinCliprdrError> {
        // SAFETY: We own the clipboard at moment of method invocation, therefore it is safe to
        // call `SetClipboardData`.
        let result = unsafe { SetClipboardData(format.value(), HANDLE(core::ptr::null_mut())) };

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
