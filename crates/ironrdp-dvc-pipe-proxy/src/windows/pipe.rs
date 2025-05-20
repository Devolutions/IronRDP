use core::ops::DerefMut;
use core::pin::Pin;

use windows::Win32::Foundation::{ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE};
use windows::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_WAIT,
};
use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};

use crate::windows::{ensure_overlapped_io_result, Event, Handle, WideString, WindowsError};

const PIPE_INSTANCES: u32 = 2;
const PIPE_BUFFER_SIZE: u32 = 64 * 1024; // 64KB
const DEFAULT_PIPE_TIMEOUT: u32 = 10_000; // 10 seconds

/// RAII wrapper for WinAPI named pipe server.
#[derive(Debug)]
pub(crate) struct MessagePipeServer {
    handle: Handle,
    connected: bool,
}

/// SAFETY: It is safe to send pipe HANDLE between threads.
unsafe impl Send for MessagePipeServer {}

impl MessagePipeServer {
    /// Creates a new named pipe server.
    pub(crate) fn new(name: &str) -> Result<Self, WindowsError> {
        let lpname = WideString::new(name);

        // Create a named pipe with the specified name.

        // SAFETY: lpname is a valid pointer to a null-terminated wide string.
        let handle = unsafe {
            CreateNamedPipeW(
                lpname.as_pcwstr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                PIPE_INSTANCES,
                PIPE_BUFFER_SIZE,
                PIPE_BUFFER_SIZE,
                DEFAULT_PIPE_TIMEOUT,
                None,
            )
        };

        // `windows` crate API inconsistency: CreateNamedPipeW returns invalid handle on error
        // instead of Result::Err.
        if handle.is_invalid() {
            return Err(WindowsError::CreateNamedPipe(windows::core::Error::from_win32()));
        }

        // SAFETY: Handle is valid and we are the owner of the handle.
        let handle = unsafe { Handle::new_owned(handle)? };

        Ok(Self {
            handle,
            connected: false,
        })
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.handle.raw()
    }

    /// Initializes context for overlapped connect operation.
    pub(crate) fn prepare_connect_overlapped(&mut self) -> Result<OverlappedPipeConnectCtx<'_>, WindowsError> {
        OverlappedPipeConnectCtx::new(self)
    }

    /// Initializes context for overlapped read operation.
    pub(crate) fn prepare_read_overlapped(
        &self,
        buffer_size: usize,
    ) -> Result<OverlappedPipeReadCtx<'_>, WindowsError> {
        OverlappedPipeReadCtx::new(self, buffer_size)
    }

    /// Initializes context for overlapped write operation.
    pub(crate) fn prepare_write_overlapped(&self, data: Vec<u8>) -> Result<OverlappedWriteCtx<'_>, WindowsError> {
        OverlappedWriteCtx::new(self, data)
    }
}

pub(crate) struct OverlappedPipeConnectCtx<'a> {
    pipe: &'a mut MessagePipeServer,
    overlapped: Pin<Box<OVERLAPPED>>,
    event: Event,
}

impl<'a> OverlappedPipeConnectCtx<'a> {
    fn new(pipe: &'a mut MessagePipeServer) -> Result<Self, WindowsError> {
        let event = Event::new_unnamed()?;

        let overlapped = Box::pin(OVERLAPPED {
            hEvent: event.raw(),
            ..Default::default()
        });

        Ok(Self {
            pipe,
            overlapped,
            event,
        })
    }

    pub(crate) fn event(&self) -> &Event {
        &self.event
    }

    /// Connects to the named pipe server.
    /// Returns `true` if pipe is already connected prior to this call and no additional
    /// overlapped io is needed. If `false` is returned, the caller should call `get_result()` to
    /// after waiting for the event to be signaled.
    pub(crate) fn overlapped_connect(&mut self) -> Result<bool, WindowsError> {
        // SAFETY: The handle is valid and we are the owner of the handle.
        let result = unsafe { ConnectNamedPipe(self.pipe.raw(), Some(self.overlapped.deref_mut() as *mut _)) };

        match result {
            Ok(()) => {
                self.pipe.connected = true;
                Ok(true)
            }
            Err(error) => {
                if error.code() == ERROR_PIPE_CONNECTED.to_hresult() {
                    // The pipe is already connected.
                    self.pipe.connected = true;
                    Ok(true)
                } else if error.code() == ERROR_IO_PENDING.to_hresult() {
                    // Overlapped I/O is pending.
                    Ok(false)
                } else {
                    // Connection failed.
                    Err(WindowsError::OverlappedConnect(error))
                }
            }
        }
    }

    pub(crate) fn get_result(&mut self) -> Result<(), WindowsError> {
        let mut bytes_read = 0u32;

        // SAFETY: The handle is valid and we are the owner of the handle.
        unsafe {
            GetOverlappedResult(
                self.pipe.raw(),
                self.overlapped.deref_mut() as *mut _,
                &mut bytes_read as *mut u32,
                false,
            )
            .map_err(WindowsError::OverlappedConnect)?
        };

        self.pipe.connected = true;

        Ok(())
    }
}

pub(crate) struct OverlappedPipeReadCtx<'a> {
    pipe: &'a MessagePipeServer,
    buffer: Vec<u8>,
    overlapped: Pin<Box<OVERLAPPED>>,
    event: Event,
}

impl<'a> OverlappedPipeReadCtx<'a> {
    fn new(pipe: &'a MessagePipeServer, buffer_size: usize) -> Result<Self, WindowsError> {
        let event = Event::new_unnamed()?;

        let overlapped = Box::pin(OVERLAPPED {
            hEvent: event.raw(),
            ..Default::default()
        });

        Ok(Self {
            pipe,
            buffer: vec![0; buffer_size],
            overlapped,
            event,
        })
    }

    pub(crate) fn event(&self) -> &Event {
        &self.event
    }

    pub(crate) fn overlapped_read(&mut self) -> Result<(), WindowsError> {
        // SAFETY: hfile is a valid handle to a named pipe; lpBuffer
        // is a valid pointer which should be alive until the operation is completed;
        let result = unsafe {
            ReadFile(
                self.pipe.raw(),
                Some(self.buffer.as_mut_slice()),
                None,
                Some(self.overlapped.deref_mut() as *mut _),
            )
        };

        ensure_overlapped_io_result(result)?.map_err(WindowsError::OverlappedRead)
    }

    pub(crate) fn get_result(&mut self) -> Result<&[u8], WindowsError> {
        let mut bytes_read = 0u32;

        // SAFETY: The handle is valid and we are the owner of the handle.
        unsafe {
            GetOverlappedResult(
                self.pipe.raw(),
                self.overlapped.deref_mut() as *mut _,
                &mut bytes_read as *mut u32,
                false,
            )
            .map_err(WindowsError::OverlappedRead)?
        };

        Ok(&self.buffer[..bytes_read as usize])
    }
}

pub(crate) struct OverlappedWriteCtx<'a> {
    pipe: &'a MessagePipeServer,
    data: Vec<u8>,
    overlapped: Pin<Box<OVERLAPPED>>,
    event: Event,
}

impl<'a> OverlappedWriteCtx<'a> {
    fn new(pipe: &'a MessagePipeServer, data: Vec<u8>) -> Result<Self, WindowsError> {
        let event = Event::new_unnamed()?;

        let mut overlapped = Box::pin(OVERLAPPED {
            hEvent: event.raw(),
            ..Default::default()
        });

        // Set write mode to append
        overlapped.Anonymous.Anonymous.Offset = 0xFFFFFFFF;
        overlapped.Anonymous.Anonymous.OffsetHigh = 0xFFFFFFFF;

        Ok(Self {
            pipe,
            data,
            overlapped,
            event,
        })
    }

    pub(crate) fn event(&self) -> &Event {
        &self.event
    }

    pub(crate) fn overlapped_write(&mut self) -> Result<(), WindowsError> {
        // SAFETY: hfile is a valid handle to a named pipe; lpBuffer
        // is a valid pointer which should be alive until the operation is completed;
        let result = unsafe {
            WriteFile(
                self.pipe.raw(),
                Some(&self.data),
                None,
                Some(self.overlapped.deref_mut() as *mut _),
            )
        };

        ensure_overlapped_io_result(result)?.map_err(WindowsError::OverlappedWrite)
    }

    pub(crate) fn get_result(&mut self) -> Result<u32, WindowsError> {
        let mut bytes_written = 0u32;

        // SAFETY: The handle is valid and we are the owner of the handle.
        unsafe {
            GetOverlappedResult(
                self.pipe.raw(),
                self.overlapped.deref_mut() as *const _,
                &mut bytes_written as *mut u32,
                true,
            )
            .map_err(WindowsError::OverlappedWrite)?;
        };

        Ok(bytes_written)
    }
}
