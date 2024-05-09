#![allow(clippy::unused_self)] // We want to keep the signature of the function stay the same between windows and non-windows

use super::ffi::CliprdrBackendFactory;
use crate::error::ffi::IronRdpError;
#[cfg(not(windows))]
use crate::error::WrongOSError;
#[cfg(not(windows))]
use ironrdp_cliprdr_native as _; // avoid linter error, stub clipboard will be used in later commit

/*
    Why are we creating a WinCliprdrInner struct and implement differently?

    1. We want to keep the FFI interface align with generated bindings.
    2. ironrdp_cliprdr_native::WinClipboard only compiles if the target is windows.
    3. We do not want to put any conditional compilation in the ffi module.

    Hence we create a WinCliprdrInner struct and implement it differently based on the target platform.
    and throw WrongOSError if the target platform is not windows.
*/
#[diplomat::bridge]
pub mod ffi {

    use crate::clipboard::ffi::CliprdrBackendFactory;
    use crate::clipboard::message::ffi::ClipboardMessage;
    use crate::error::ffi::IronRdpError;
    use crate::error::{PointWidth, WrongPointWidthError};

    use super::WinCliprdrInner;

    #[diplomat::opaque]
    pub struct WinCliprdr(WinCliprdrInner);

    impl WinCliprdr {
        pub fn new_32bit(hwnd: u32) -> Result<Box<WinCliprdr>, Box<IronRdpError>> {
            let pointer =
                isize::try_from(hwnd).map_err(|_| WrongPointWidthError::expected_width(PointWidth::Width32))?;

            WinCliprdrInner::new(pointer).map(WinCliprdr).map(Box::new)
        }

        pub fn new_64bit(hwnd: u64) -> Result<Box<WinCliprdr>, Box<IronRdpError>> {
            let pointer =
                isize::try_from(hwnd).map_err(|_| WrongPointWidthError::expected_width(PointWidth::Width64))?;

            WinCliprdrInner::new(pointer).map(WinCliprdr).map(Box::new)
        }

        pub fn next_clipboard_message(&self) -> Result<Option<Box<ClipboardMessage>>, Box<IronRdpError>> {
            Ok(self.0.next_clipboard_message()?.map(ClipboardMessage).map(Box::new))
        }

        pub fn next_clipboard_message_blocking(&self) -> Result<Box<ClipboardMessage>, Box<IronRdpError>> {
            self.0
                .next_clipboard_message_blocking()
                .map(ClipboardMessage)
                .map(Box::new)
        }

        pub fn backend_factory(&self) -> Result<Box<CliprdrBackendFactory>, Box<IronRdpError>> {
            self.0.backend_factory().map(Box::new)
        }
    }
}

#[cfg(not(windows))]
pub struct WinCliprdrInner;

#[cfg(not(windows))]
impl WinCliprdrInner {
    fn new(_hwnd: isize) -> Result<WinCliprdrInner, Box<IronRdpError>> {
        Err(WrongOSError::expected_platform("windows")
            .with_custom_message("WinCliprdr only support windows")
            .into())
    }

    fn next_clipboard_message(&self) -> Result<Option<ironrdp::cliprdr::backend::ClipboardMessage>, Box<IronRdpError>> {
        Err(WrongOSError::expected_platform("windows")
            .with_custom_message("WinCliprdr only support windows")
            .into())
    }

    fn backend_factory(&self) -> Result<CliprdrBackendFactory, Box<IronRdpError>> {
        Err(WrongOSError::expected_platform("windows")
            .with_custom_message("WinCliprdr only support windows")
            .into())
    }

    fn next_clipboard_message_blocking(
        &self,
    ) -> Result<ironrdp::cliprdr::backend::ClipboardMessage, Box<IronRdpError>> {
        Err(WrongOSError::expected_platform("windows")
            .with_custom_message("WinCliprdr only support windows")
            .into())
    }
}

#[cfg(windows)]
pub struct WinCliprdrInner {
    pub clipboard: ironrdp_cliprdr_native::WinClipboard,
    pub receiver: std::sync::mpsc::Receiver<ironrdp::cliprdr::backend::ClipboardMessage>,
}

#[cfg(windows)]
impl WinCliprdrInner {
    fn new(hwnd: isize) -> Result<WinCliprdrInner, Box<IronRdpError>> {
        use windows::Win32::Foundation::HWND;

        let (sender, receiver) = std::sync::mpsc::channel();

        let proxy = crate::clipboard::FfiClipbarodMessageProxy { sender };

        // SAFETY: `hwnd` must be a valid window handle
        let clipboard = unsafe { ironrdp_cliprdr_native::WinClipboard::new(HWND(hwnd), proxy) }?;

        Ok(WinCliprdrInner { clipboard, receiver })
    }

    fn next_clipboard_message(&self) -> Result<Option<ironrdp::cliprdr::backend::ClipboardMessage>, Box<IronRdpError>> {
        Ok(self.receiver.try_recv().ok())
    }

    fn backend_factory(&self) -> Result<CliprdrBackendFactory, Box<IronRdpError>> {
        Ok(CliprdrBackendFactory(self.clipboard.backend_factory()))
    }

    fn next_clipboard_message_blocking(
        &self,
    ) -> Result<ironrdp::cliprdr::backend::ClipboardMessage, Box<IronRdpError>> {
        Ok(self
            .receiver
            .recv()
            .map_err(|_| "Failed to receive clipboard message")?)
    }
}
