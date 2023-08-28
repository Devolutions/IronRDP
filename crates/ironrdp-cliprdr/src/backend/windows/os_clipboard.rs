use crate::backend::windows::WinCliprdrError;
use crate::pdu::{ClipboardFormat, ClipboardFormatId, ClipboardFormatName};

use tracing::error;
use winapi::shared::windef::HWND;
use winapi::shared::winerror::{ERROR_ACCESS_DENIED, ERROR_SUCCESS};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::winuser::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardFormatNameW, OpenClipboard, SetClipboardData,
};

/// Safe wrapper around windows. Clipboard is automatically closed on drop.
pub struct OwnedOsClipboard;

impl OwnedOsClipboard {
    pub fn new(window: HWND) -> Result<Self, WinCliprdrError> {
        if unsafe { OpenClipboard(window) } == 0 {
            let last_error = unsafe { GetLastError() };

            // Retryable error
            if last_error == ERROR_ACCESS_DENIED {
                return Err(WinCliprdrError::ClipboardAccessDenied);
            }

            // Unknown critical error
            return Err(WinCliprdrError::ClipboardOpen);
        }

        Ok(Self)
    }

    /// Enumerates all available formats in the current clipboard.
    pub fn enum_available_formats(&self) -> Result<Vec<ClipboardFormat>, WinCliprdrError> {
        const DEFAULT_FORMATS_CAPACITY: usize = 16;
        // Sane default for format name name. If format names is longer than this,
        // `GetClipboardFormatNameW` will truncate it.
        const MAX_FORMAT_NAME_LENGTH: usize = 256;

        let mut formats = Vec::with_capacity(DEFAULT_FORMATS_CAPACITY);

        let mut raw_format = unsafe { EnumClipboardFormats(0) };
        let mut format_name_w = [0u16; MAX_FORMAT_NAME_LENGTH];

        while raw_format != 0 {
            let format_id = ClipboardFormatId::new(raw_format);

            // Get format name for predefined formats
            let format = if !format_id.is_standard() {
                let read_chars = unsafe {
                    GetClipboardFormatNameW(raw_format, format_name_w.as_mut_ptr(), format_name_w.len() as i32)
                } as usize;

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

            // Next available format in clipboard
            raw_format = unsafe { EnumClipboardFormats(raw_format) };
        }

        if unsafe { GetLastError() } != ERROR_SUCCESS {
            return Err(WinCliprdrError::FormatsEnumeration);
        }

        Ok(formats)
    }

    pub fn clear(&mut self) -> Result<(), WinCliprdrError> {
        // We need to empty clipboard before setting any delay-rendered data
        if unsafe { EmptyClipboard() } == 0 {
            return Err(WinCliprdrError::ClipboardEmpty);
        }

        Ok(())
    }

    pub fn delay_render(&mut self, format: ClipboardFormatId) -> Result<(), WinCliprdrError> {
        let result = unsafe { SetClipboardData(format.value(), 0 as _) };
        if result.is_null() {
            let error = unsafe { GetLastError() };
            if error != ERROR_SUCCESS {
                error!("Failed to delayed clipboard rendering for format {}", format.value());
                return Err(WinCliprdrError::SetClipboardData);
            }
        }

        Ok(())
    }
}

impl Drop for OwnedOsClipboard {
    fn drop(&mut self) {
        if unsafe { CloseClipboard() } == 0 {
            error!("Failed to close Windows clipboard");
        }
    }
}
