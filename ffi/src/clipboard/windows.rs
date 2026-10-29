#![allow(clippy::unused_self)] // We want to keep the signature of the function stay the same between windows and non-windows

#[cfg(not(windows))]
use ironrdp_cliprdr_native as _;

use super::ffi::CliprdrBackendFactory;
#[cfg(not(windows))]
use crate::error::WrongOSError;
use crate::error::ffi::IronRdpError; // avoid linter error, stub clipboard will be used in later commit

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

    use super::WinCliprdrInner;
    use crate::clipboard::ffi::CliprdrBackendFactory;
    use crate::clipboard::message::ffi::ClipboardMessage;
    use crate::error::ffi::IronRdpError;

    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque]
    pub struct WinCliprdr(WinCliprdrInner);

    impl WinCliprdr {
        /// Creates the Windows clipboard backend without file transfer.
        pub fn new() -> Result<Box<WinCliprdr>, Box<IronRdpError>> {
            WinCliprdrInner::new(false).map(WinCliprdr).map(Box::new)
        }

        /// Creates the Windows clipboard backend, with clipboard file transfer enabled or not.
        ///
        /// With file transfer, the backend also produces `SendInitiateFileCopy`,
        /// `SendFileContentsRequest` and `SendFileContentsResponse` messages: hand every message
        /// to `ActiveStage::submit_clipboard_message`, and call
        /// `ActiveStage::drive_clipboard_timeouts` every few seconds. Pasting remote files needs
        /// this to be called on an STA thread with a message loop, such as the UI thread.
        pub fn new_with_file_transfer(file_transfer: bool) -> Result<Box<WinCliprdr>, Box<IronRdpError>> {
            WinCliprdrInner::new(file_transfer).map(WinCliprdr).map(Box::new)
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

        /// Waits up to `timeout_ms` milliseconds for the next message; `None` if none came.
        pub fn next_clipboard_message_timeout(
            &self,
            timeout_ms: u32,
        ) -> Result<Option<Box<ClipboardMessage>>, Box<IronRdpError>> {
            Ok(self
                .0
                .next_clipboard_message_timeout(timeout_ms)?
                .map(ClipboardMessage)
                .map(Box::new))
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
    fn new(_file_transfer: bool) -> Result<WinCliprdrInner, Box<IronRdpError>> {
        Err(WrongOSError::expected_platform("windows")
            .with_custom_message("WinCliprdr only support windows")
            .into())
    }

    fn next_clipboard_message(&self) -> Result<Option<ironrdp::cliprdr::backend::ClipboardMessage>, Box<IronRdpError>> {
        Err(WrongOSError::expected_platform("windows")
            .with_custom_message("WinCliprdr only support windows")
            .into())
    }

    fn next_clipboard_message_timeout(
        &self,
        _timeout_ms: u32,
    ) -> Result<Option<ironrdp::cliprdr::backend::ClipboardMessage>, Box<IronRdpError>> {
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
    fn new(file_transfer: bool) -> Result<WinCliprdrInner, Box<IronRdpError>> {
        let (sender, receiver) = std::sync::mpsc::channel();

        let proxy = crate::clipboard::FfiClipboardMessageProxy { sender };

        // Off by default: embedders that don't route the file stream messages would leave the
        // remote waiting on them.
        let clipboard = ironrdp_cliprdr_native::WinClipboard::with_file_transfer(proxy, file_transfer)?;

        Ok(WinCliprdrInner { clipboard, receiver })
    }

    fn next_clipboard_message(&self) -> Result<Option<ironrdp::cliprdr::backend::ClipboardMessage>, Box<IronRdpError>> {
        match self.receiver.try_recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("clipboard message channel disconnected".into()),
        }
    }

    fn next_clipboard_message_timeout(
        &self,
        timeout_ms: u32,
    ) -> Result<Option<ironrdp::cliprdr::backend::ClipboardMessage>, Box<IronRdpError>> {
        let timeout = core::time::Duration::from_millis(u64::from(timeout_ms));
        match self.receiver.recv_timeout(timeout) {
            Ok(msg) => Ok(Some(msg)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err("clipboard message channel disconnected".into())
            }
        }
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
