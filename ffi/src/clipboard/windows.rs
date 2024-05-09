use crate::error::ffi::IronRdpError;

use self::ffi::WinCliprdr;

#[diplomat::bridge]
pub mod ffi {

    use crate::clipboard::ffi::CliprdrBackendFactory;
    use crate::clipboard::message::ffi::ClipboardMessage;
    use crate::error::ffi::IronRdpError;
    use crate::error::{PointWidth, WrongPointWidthError};

    #[diplomat::opaque]
    pub struct WinCliprdr {
        pub clipboard: ironrdp_cliprdr_native::WinClipboard,
        pub receiver: std::sync::mpsc::Receiver<ironrdp::cliprdr::backend::ClipboardMessage>,
    }

    impl WinCliprdr {
        pub fn new_32bit(hwnd: u32) -> Result<Box<WinCliprdr>, Box<IronRdpError>> {
            let pointer =
                isize::try_from(hwnd).map_err(|_| WrongPointWidthError::expected_width(PointWidth::Width32))?;

            WinCliprdr::new(pointer)
        }

        pub fn new_64bit(hwnd: u64) -> Result<Box<WinCliprdr>, Box<IronRdpError>> {
            let pointer =
                isize::try_from(hwnd).map_err(|_| WrongPointWidthError::expected_width(PointWidth::Width64))?;

            WinCliprdr::new(pointer)
        }

        pub fn next_clipboard_message(&self) -> Option<Box<ClipboardMessage>> {
            self.receiver.try_recv().ok().map(ClipboardMessage).map(Box::new)
        }

        pub fn next_clipboard_message_blocking(&self) -> Result<Box<ClipboardMessage>, Box<IronRdpError>> {
            self.receiver
                .recv()
                .map(ClipboardMessage)
                .map(Box::new)
                .map_err(|_| "receiver closed".into())
        }

        pub fn backend_factory(&self) -> Box<CliprdrBackendFactory> {
            Box::new(CliprdrBackendFactory(self.clipboard.backend_factory()))
        }
    }
}

impl WinCliprdr {
    fn new(hwnd: isize) -> Result<Box<WinCliprdr>, Box<IronRdpError>> {
        #[cfg(not(windows))]
        {
            use crate::error::WrongOSError;
            return Err(WrongOSError::expected_platform("windows")
                .with_custom_message("WinCliprdr only support windows")
                .into());
        }

        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;

            let (sender, receiver) = std::sync::mpsc::channel();

            let proxy = crate::clipboard::FfiClipbarodMessageProxy { sender };

            // SAFETY: `hwnd` must be a valid window handle
            let clipboard = unsafe { ironrdp_cliprdr_native::WinClipboard::new(HWND(hwnd), proxy) }?;

            Ok(Box::new(WinCliprdr { clipboard, receiver }))
        }
    }
}
