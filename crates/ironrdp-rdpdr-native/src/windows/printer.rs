use core::ffi::c_void;
use core::fmt;
use core::mem::size_of;
use core::slice;
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpdr::RdpdrPrinter;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    DeviceCloseResponse, DeviceCreateRequest, DeviceCreateResponse, DeviceIoResponse, DeviceWriteRequest,
    DeviceWriteResponse, Information, NtStatus, PrinterIoRequest,
};
use ironrdp_svc::SvcMessage;
use tracing::{debug, warn};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, GetLastError};
use windows::Win32::Graphics::Printing::{
    AbortPrinter, ClosePrinter, DOC_INFO_1W, EndDocPrinter, GetDefaultPrinterW, GetPrinterW, OpenPrinterW,
    PRINTER_ATTRIBUTE_NETWORK, PRINTER_HANDLE, PRINTER_INFO_2W, StartDocPrinterW, WritePrinter,
};
use windows::core::{Error, HRESULT, PCWSTR, PWSTR};

const MAX_PRINTER_NAME_UTF16_UNITS: usize = 1_024;
const MAX_PRINTER_METADATA_BYTES: usize = 1024 * 1024;
const MAX_OPEN_PRINT_JOBS: usize = 16;
const MAX_PRINT_JOB_BYTES: usize = 128 * 1024 * 1024;
const PRINTER_COMMAND_QUEUE_CAPACITY: usize = 16;
const PRINTER_RESET_WAIT: Duration = Duration::from_millis(100);

/// Guard that releases a host lifetime pin while atomically exiting the worker thread.
pub trait RdpdrWorkerThreadGuard: Send {
    /// Releases the host lifetime pin and exits the current worker thread atomically.
    fn exit(self: Box<Self>) -> !;
}

/// Result of acquiring a host lifetime guard before spawning an RDPDR worker.
pub type RdpdrWorkerThreadGuardResult =
    Result<Box<dyn RdpdrWorkerThreadGuard>, Box<dyn core::error::Error + Send + Sync>>;

/// Acquires a host lifetime guard before a native RDPDR worker thread starts.
#[derive(Clone, Copy, Debug)]
pub struct RdpdrWorkerThreadHooks {
    acquire: fn() -> RdpdrWorkerThreadGuardResult,
}

impl RdpdrWorkerThreadHooks {
    /// Creates a worker-lifetime hook.
    pub fn new(acquire: fn() -> RdpdrWorkerThreadGuardResult) -> Self {
        Self { acquire }
    }

    fn acquire(&self) -> RdpdrWorkerThreadGuardResult {
        (self.acquire)()
    }
}

pub(super) fn discover_default_printer(device_id: u32) -> Result<Option<RdpdrPrinter>, DefaultPrinterError> {
    let queue_name = match default_printer_name()? {
        Some(name) => name,
        None => return Ok(None),
    };
    let (driver_name, network) = printer_driver(&queue_name)?;

    validate_printer_name(&queue_name)?;
    validate_printer_name(&driver_name)?;

    Ok(Some(
        RdpdrPrinter::new(device_id, queue_name, driver_name).with_network(network),
    ))
}

#[derive(Debug)]
pub(super) enum DefaultPrinterError {
    Windows(Error),
    InvalidMetadata(&'static str),
    InvalidUtf16,
}

impl fmt::Display for DefaultPrinterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Windows(error) => write!(f, "query Windows default printer: {error}"),
            Self::InvalidMetadata(message) => f.write_str(message),
            Self::InvalidUtf16 => f.write_str("default printer metadata contains invalid UTF-16"),
        }
    }
}

impl core::error::Error for DefaultPrinterError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Windows(error) => Some(error),
            Self::InvalidMetadata(_) | Self::InvalidUtf16 => None,
        }
    }
}

impl From<Error> for DefaultPrinterError {
    fn from(error: Error) -> Self {
        Self::Windows(error)
    }
}

fn validate_printer_name(value: &str) -> Result<(), DefaultPrinterError> {
    let units = value.encode_utf16().count();
    if units == 0 {
        return Err(DefaultPrinterError::InvalidMetadata(
            "default printer metadata must not be empty",
        ));
    }
    if units > MAX_PRINTER_NAME_UTF16_UNITS {
        return Err(DefaultPrinterError::InvalidMetadata(
            "default printer metadata exceeds the supported length",
        ));
    }
    if value.contains('\0') {
        return Err(DefaultPrinterError::InvalidMetadata(
            "default printer metadata must not contain NUL",
        ));
    }
    Ok(())
}

fn default_printer_name() -> Result<Option<String>, DefaultPrinterError> {
    let mut length = 0u32;
    // SAFETY: A null output buffer is the documented size-probe form and `length` is writable.
    let result = unsafe { GetDefaultPrinterW(None, &mut length) };
    if result.as_bool() {
        return Err(DefaultPrinterError::InvalidMetadata(
            "default printer size probe unexpectedly succeeded",
        ));
    }

    // SAFETY: GetLastError only reads the calling thread's error state immediately after the failed probe.
    let error = unsafe { GetLastError() };
    if error == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if error != ERROR_INSUFFICIENT_BUFFER {
        return Err(Error::from_hresult(HRESULT::from_win32(error.0)).into());
    }

    let length = usize::try_from(length)
        .map_err(|_| DefaultPrinterError::InvalidMetadata("default printer name length does not fit usize"))?;
    if length == 0 || length > MAX_PRINTER_NAME_UTF16_UNITS + 1 {
        return Err(DefaultPrinterError::InvalidMetadata(
            "default printer name has an invalid length",
        ));
    }

    let mut buffer = vec![0u16; length];
    let mut actual = u32::try_from(buffer.len()).expect("bounded printer name length fits u32");
    // SAFETY: `buffer` is writable for `actual` UTF-16 units and remains alive for the call.
    if !unsafe { GetDefaultPrinterW(Some(PWSTR(buffer.as_mut_ptr())), &mut actual) }.as_bool() {
        return Err(last_error().into());
    }

    let actual = usize::try_from(actual)
        .map_err(|_| DefaultPrinterError::InvalidMetadata("default printer name length does not fit usize"))?;
    if actual == 0 || actual > buffer.len() || buffer[actual - 1] != 0 || buffer[..actual - 1].contains(&0) {
        return Err(DefaultPrinterError::InvalidMetadata(
            "default printer name is not a single NUL-terminated string",
        ));
    }

    String::from_utf16(&buffer[..actual - 1])
        .map(Some)
        .map_err(|_| DefaultPrinterError::InvalidUtf16)
}

fn printer_driver(queue_name: &str) -> Result<(String, bool), DefaultPrinterError> {
    let queue_name = wide_nul(queue_name);
    let handle = PrinterHandle::open(&queue_name)?;
    let mut needed = 0u32;
    // SAFETY: `handle` is valid and a null buffer is the documented size-probe form.
    let probe_error = match unsafe { GetPrinterW(handle.raw(), 2, None, &mut needed) } {
        Ok(()) => {
            return Err(DefaultPrinterError::InvalidMetadata(
                "printer metadata size probe unexpectedly succeeded",
            ));
        }
        Err(error) => error,
    };
    let buffer_length = usize::try_from(needed)
        .map_err(|_| DefaultPrinterError::InvalidMetadata("printer metadata length does not fit usize"))?;
    if buffer_length < size_of::<PRINTER_INFO_2W>() || buffer_length > MAX_PRINTER_METADATA_BYTES {
        return Err(if buffer_length == 0 {
            DefaultPrinterError::Windows(probe_error)
        } else {
            DefaultPrinterError::InvalidMetadata("printer metadata has an invalid length")
        });
    }

    let word_count = buffer_length.div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; word_count];
    // SAFETY: `storage` is contiguous, writable, and spans at least `buffer_length` bytes.
    let bytes = unsafe { slice::from_raw_parts_mut(storage.as_mut_ptr().cast::<u8>(), buffer_length) };
    // SAFETY: `handle` is valid and `bytes` is a writable buffer of the size returned by the probe.
    unsafe { GetPrinterW(handle.raw(), 2, Some(bytes), &mut needed) }?;

    // SAFETY: GetPrinterW level 2 initialized the aligned buffer with a PRINTER_INFO_2W.
    let info = unsafe { &*storage.as_ptr().cast::<PRINTER_INFO_2W>() };
    if info.pDriverName.is_null() {
        return Err(DefaultPrinterError::InvalidMetadata(
            "default printer has no driver name",
        ));
    }
    // SAFETY: GetPrinterW returned a NUL-terminated driver-name pointer valid while `storage` is alive.
    let driver_name = unsafe { info.pDriverName.to_string() }.map_err(|_| DefaultPrinterError::InvalidUtf16)?;
    Ok((driver_name, info.Attributes & PRINTER_ATTRIBUTE_NETWORK != 0))
}

fn wide_nul(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(core::iter::once(0)).collect()
}

fn last_error() -> Error {
    // SAFETY: GetLastError only reads the calling thread's error state.
    let error = unsafe { GetLastError() };
    Error::from_hresult(HRESULT::from_win32(error.0))
}

struct PrinterHandle(PRINTER_HANDLE);

// SAFETY: PrinterHandle is exclusively owned and all access remains serialized on the worker thread.
unsafe impl Send for PrinterHandle {}

impl PrinterHandle {
    fn open(queue_name: &[u16]) -> Result<Self, Error> {
        let mut handle = PRINTER_HANDLE::default();
        // SAFETY: `queue_name` is NUL-terminated and `handle` is writable.
        unsafe { OpenPrinterW(PCWSTR(queue_name.as_ptr()), &mut handle, None) }?;
        Ok(Self(handle))
    }

    fn raw(&self) -> PRINTER_HANDLE {
        self.0
    }
}

impl Drop for PrinterHandle {
    fn drop(&mut self) {
        // SAFETY: This object owns the printer handle and closes it exactly once.
        if let Err(error) = unsafe { ClosePrinter(self.0) } {
            warn!(%error, "Failed to close Windows printer handle");
        }
    }
}

struct Win32PrintJob {
    handle: PrinterHandle,
    active: bool,
}

impl Drop for Win32PrintJob {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: The handle owns an active StartDocPrinter job.
            if !unsafe { AbortPrinter(self.handle.raw()) }.as_bool() {
                warn!(error = %last_error(), "Failed to abort Windows print job");
            }
        }
    }
}

type SpoolerResult = Result<(), Box<dyn core::error::Error + Send>>;

trait PrintSpooler: Send + 'static {
    fn create(&mut self, file_id: u32, printer: &RdpdrPrinter) -> SpoolerResult;
    fn write(&mut self, file_id: u32, data: &[u8]) -> SpoolerResult;
    fn close(&mut self, file_id: u32) -> SpoolerResult;
    fn abort(&mut self, file_id: u32);
    fn reset(&mut self);
}

#[derive(Default)]
struct Win32PrintSpooler {
    jobs: HashMap<u32, Win32PrintJob>,
}

impl PrintSpooler for Win32PrintSpooler {
    fn create(&mut self, file_id: u32, printer: &RdpdrPrinter) -> SpoolerResult {
        let queue_name = wide_nul(printer.name());
        let handle =
            PrinterHandle::open(&queue_name).map_err(|error| Box::new(error) as Box<dyn core::error::Error + Send>)?;
        let mut document_name = wide_nul("Remote Desktop Document");
        let mut data_type = wide_nul("RAW");
        let document = DOC_INFO_1W {
            pDocName: PWSTR(document_name.as_mut_ptr()),
            pOutputFile: PWSTR::null(),
            pDatatype: PWSTR(data_type.as_mut_ptr()),
        };
        // SAFETY: `handle` is valid and all DOC_INFO_1W strings remain alive for the call.
        let job_id = unsafe { StartDocPrinterW(handle.raw(), 1, &document) };
        if job_id == 0 {
            return Err(Box::new(last_error()));
        }

        self.jobs.insert(file_id, Win32PrintJob { handle, active: true });
        Ok(())
    }

    fn write(&mut self, file_id: u32, data: &[u8]) -> SpoolerResult {
        let job = self
            .jobs
            .get(&file_id)
            .ok_or_else(|| Box::new(InvalidPrintHandle) as Box<dyn core::error::Error + Send>)?;
        let length = u32::try_from(data.len())
            .map_err(|_| Box::new(PrintChunkTooLarge) as Box<dyn core::error::Error + Send>)?;
        let mut written = 0u32;
        // SAFETY: The job handle is active and `data` is readable for `length` bytes.
        if !unsafe { WritePrinter(job.handle.raw(), data.as_ptr().cast::<c_void>(), length, &mut written) }.as_bool() {
            return Err(Box::new(last_error()));
        }
        if written != length {
            return Err(Box::new(ShortPrinterWrite {
                expected: length,
                actual: written,
            }));
        }
        Ok(())
    }

    fn close(&mut self, file_id: u32) -> SpoolerResult {
        let mut job = self
            .jobs
            .remove(&file_id)
            .ok_or_else(|| Box::new(InvalidPrintHandle) as Box<dyn core::error::Error + Send>)?;
        // SAFETY: The job handle owns an active StartDocPrinter job.
        if !unsafe { EndDocPrinter(job.handle.raw()) }.as_bool() {
            return Err(Box::new(last_error()));
        }
        job.active = false;
        Ok(())
    }

    fn abort(&mut self, file_id: u32) {
        self.jobs.remove(&file_id);
    }

    fn reset(&mut self) {
        self.jobs.clear();
    }
}

#[derive(Debug)]
struct InvalidPrintHandle;

impl fmt::Display for InvalidPrintHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid print handle")
    }
}

impl core::error::Error for InvalidPrintHandle {}

#[derive(Debug)]
struct PrintChunkTooLarge;

impl fmt::Display for PrintChunkTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("print chunk exceeds the Windows spooler size limit")
    }
}

impl core::error::Error for PrintChunkTooLarge {}

#[derive(Debug)]
struct ShortPrinterWrite {
    expected: u32,
    actual: u32,
}

impl fmt::Display for ShortPrinterWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Windows spooler accepted {} of {} print bytes",
            self.actual, self.expected
        )
    }
}

impl core::error::Error for ShortPrinterWrite {}

enum PrinterCommand {
    Create { request: DeviceCreateRequest, file_id: u32 },
    Write(DeviceWriteRequest),
    Close(ironrdp_rdpdr::pdu::efs::DeviceCloseRequest),
}

#[derive(Debug)]
pub(super) struct PrinterWorker {
    printer: RdpdrPrinter,
    commands: Option<SyncSender<PrinterCommand>>,
    completions: Receiver<SvcMessage>,
    next_file_id: u32,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    poisoned_file_ids: std::sync::Arc<std::sync::Mutex<HashSet<u32>>>,
    live_file_ids: std::sync::Arc<std::sync::Mutex<HashSet<u32>>>,
    finished: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    completion_disconnected: bool,
    hooks: Option<RdpdrWorkerThreadHooks>,
    guarded: bool,
}

impl PrinterWorker {
    pub(super) fn new(printer: RdpdrPrinter, hooks: Option<RdpdrWorkerThreadHooks>) -> PduResult<Self> {
        Self::with_spooler(printer, Box::<Win32PrintSpooler>::default(), hooks)
    }

    fn with_spooler(
        printer: RdpdrPrinter,
        spooler: Box<dyn PrintSpooler>,
        hooks: Option<RdpdrWorkerThreadHooks>,
    ) -> PduResult<Self> {
        let (commands, command_rx) = sync_channel(PRINTER_COMMAND_QUEUE_CAPACITY);
        let (completion_tx, completions) = std::sync::mpsc::channel();
        let worker_printer = printer.clone();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancelled = std::sync::Arc::clone(&cancelled);
        let poisoned_file_ids = std::sync::Arc::new(std::sync::Mutex::new(HashSet::new()));
        let worker_poisoned_file_ids = std::sync::Arc::clone(&poisoned_file_ids);
        let live_file_ids = std::sync::Arc::new(std::sync::Mutex::new(HashSet::new()));
        let worker_live_file_ids = std::sync::Arc::clone(&live_file_ids);
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_finished = std::sync::Arc::clone(&finished);
        let thread_guard = hooks
            .as_ref()
            .map(RdpdrWorkerThreadHooks::acquire)
            .transpose()
            .map_err(|error| {
                pdu_other_err!(
                    "acquire Windows printer worker lifetime",
                    source: std::io::Error::other(error.to_string())
                )
            })?;
        let thread = std::thread::Builder::new()
            .name("ironrdp-printer".to_owned())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    worker_loop(
                        worker_printer,
                        spooler,
                        command_rx,
                        completion_tx,
                        worker_cancelled,
                        worker_poisoned_file_ids,
                        worker_live_file_ids,
                    )
                }));
                if result.is_err() {
                    warn!("Windows printer worker panicked");
                }
                worker_finished.store(true, std::sync::atomic::Ordering::Release);
                if let Some(thread_guard) = thread_guard {
                    thread_guard.exit();
                }
            })
            .map_err(|error| pdu_other_err!("spawn Windows printer worker", source: error))?;

        Ok(Self {
            printer,
            commands: Some(commands),
            completions,
            next_file_id: 1,
            cancelled,
            poisoned_file_ids,
            live_file_ids,
            finished,
            thread: Some(thread),
            completion_disconnected: false,
            guarded: hooks.is_some(),
            hooks,
        })
    }

    pub(super) fn handle(&mut self, request: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        let command = match request {
            PrinterIoRequest::Create(request) => {
                if !request.path.is_empty() {
                    return Ok(vec![create_response(request, 0, NtStatus::INVALID_PARAMETER)]);
                }
                let file_id = self.allocate_file_id();
                PrinterCommand::Create { request, file_id }
            }
            PrinterIoRequest::Write(request) => PrinterCommand::Write(request),
            PrinterIoRequest::Close(request) => PrinterCommand::Close(request),
        };

        let commands = self
            .commands
            .as_ref()
            .ok_or_else(|| pdu_other_err!("Windows printer worker is shutting down"))?;
        match commands.try_send(command) {
            Ok(()) => Ok(Vec::new()),
            Err(TrySendError::Full(command)) => {
                warn!("Windows printer worker queue is full");
                match &command {
                    PrinterCommand::Write(request) => self.poison(request.device_io_request.file_id),
                    PrinterCommand::Close(request) => self.poison(request.device_io_request.file_id),
                    PrinterCommand::Create { .. } => {}
                }
                Ok(vec![failed_command_response(command, NtStatus::UNSUCCESSFUL)])
            }
            Err(TrySendError::Disconnected(_)) => Err(pdu_other_err!("Windows printer worker stopped unexpectedly")),
        }
    }

    pub(super) fn poll(&mut self) -> PduResult<Vec<SvcMessage>> {
        if self.completion_disconnected {
            return Ok(Vec::new());
        }
        let mut messages = Vec::new();
        loop {
            match self.completions.try_recv() {
                Ok(message) => messages.push(message),
                Err(TryRecvError::Empty) => return Ok(messages),
                Err(TryRecvError::Disconnected) => {
                    warn!("Windows printer worker completion channel disconnected");
                    self.completion_disconnected = true;
                    self.request_stop();
                    return Ok(messages);
                }
            }
        }
    }

    pub(super) fn restart(&mut self) -> PduResult<()> {
        self.request_stop();
        let deadline = Instant::now() + PRINTER_RESET_WAIT;
        while !self.finished.load(std::sync::atomic::Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        if !self.finished.load(std::sync::atomic::Ordering::Acquire) {
            return Err(pdu_other_err!(
                "Windows printer worker did not stop within reset deadline"
            ));
        }
        if let Some(thread) = self.thread.take()
            && !self.guarded
            && thread.join().is_err()
        {
            return Err(pdu_other_err!("Windows printer worker panicked during reset"));
        }
        let replacement = Self::new(self.printer.clone(), self.hooks)?;
        *self = replacement;
        Ok(())
    }

    pub(super) fn reject_write(&mut self, request: ironrdp_rdpdr::pdu::efs::DeviceIoRequest) -> Vec<SvcMessage> {
        self.poison(request.file_id);
        vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
            device_io_reply: DeviceIoResponse::new(request, NtStatus::INVALID_PARAMETER),
            length: 0,
        }))]
    }

    fn poison(&self, file_id: u32) {
        if !self
            .live_file_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&file_id)
        {
            return;
        }
        self.poisoned_file_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(file_id);
    }

    fn request_stop(&mut self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::Release);
        self.commands.take();
    }

    fn allocate_file_id(&mut self) -> u32 {
        let file_id = self.next_file_id;
        self.next_file_id = self.next_file_id.wrapping_add(1);
        if self.next_file_id == 0 {
            self.next_file_id = 1;
        }
        file_id
    }
}

impl Drop for PrinterWorker {
    fn drop(&mut self) {
        self.request_stop();
        self.thread.take();
    }
}

fn worker_loop(
    printer: RdpdrPrinter,
    mut spooler: Box<dyn PrintSpooler>,
    commands: Receiver<PrinterCommand>,
    completions: std::sync::mpsc::Sender<SvcMessage>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    poisoned_file_ids: std::sync::Arc<std::sync::Mutex<HashSet<u32>>>,
    live_file_ids: std::sync::Arc<std::sync::Mutex<HashSet<u32>>>,
) {
    let mut job_sizes = HashMap::<u32, usize>::new();
    while let Ok(command) = commands.recv() {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let poisoned = {
            let mut poisoned = poisoned_file_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            core::mem::take(&mut *poisoned)
        };
        for file_id in poisoned {
            if job_sizes.remove(&file_id).is_some() {
                spooler.abort(file_id);
                live_file_ids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&file_id);
            }
        }
        let response = match command {
            PrinterCommand::Create { request, file_id } => {
                let result = if job_sizes.len() >= MAX_OPEN_PRINT_JOBS {
                    Err("maximum open print-job count reached".to_owned())
                } else {
                    spooler.create(file_id, &printer).map_err(|error| error.to_string())
                };
                let status = match result {
                    Ok(()) => {
                        job_sizes.insert(file_id, 0);
                        live_file_ids
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(file_id);
                        debug!(
                            file_id,
                            printer = printer.name(),
                            "Started redirected Windows print job"
                        );
                        NtStatus::SUCCESS
                    }
                    Err(error) => {
                        warn!(file_id, %error, "Failed to start redirected Windows print job");
                        NtStatus::UNSUCCESSFUL
                    }
                };
                create_response(request, file_id, status)
            }
            PrinterCommand::Write(request) => {
                let file_id = request.device_io_request.file_id;
                let length = u32::try_from(request.write_data.len())
                    .expect("printer write length round-trips from the u32 wire field");
                let projected_size = job_sizes
                    .get(&file_id)
                    .and_then(|current| current.checked_add(request.write_data.len()))
                    .filter(|size| *size <= MAX_PRINT_JOB_BYTES);
                let status = match projected_size {
                    Some(size) => match spooler.write(file_id, &request.write_data) {
                        Ok(()) => {
                            job_sizes.insert(file_id, size);
                            NtStatus::SUCCESS
                        }
                        Err(error) => {
                            warn!(file_id, %error, "Failed to write redirected Windows print data");
                            spooler.abort(file_id);
                            job_sizes.remove(&file_id);
                            live_file_ids
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .remove(&file_id);
                            NtStatus::UNSUCCESSFUL
                        }
                    },
                    None => {
                        warn!(
                            file_id,
                            limit = MAX_PRINT_JOB_BYTES,
                            "Redirected print job exceeds byte limit"
                        );
                        spooler.abort(file_id);
                        job_sizes.remove(&file_id);
                        live_file_ids
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .remove(&file_id);
                        NtStatus::UNSUCCESSFUL
                    }
                };
                write_response(request, length, status)
            }
            PrinterCommand::Close(request) => {
                let file_id = request.device_io_request.file_id;
                let status = if job_sizes.remove(&file_id).is_some() {
                    live_file_ids
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&file_id);
                    match spooler.close(file_id) {
                        Ok(()) => NtStatus::SUCCESS,
                        Err(error) => {
                            warn!(file_id, %error, "Failed to finish redirected Windows print job");
                            spooler.abort(file_id);
                            NtStatus::UNSUCCESSFUL
                        }
                    }
                } else {
                    NtStatus::INVALID_HANDLE
                };
                close_response(request, status)
            }
        };

        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        if completions.send(response).is_err() {
            break;
        }
    }
    spooler.reset();
    live_file_ids
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

fn failed_command_response(command: PrinterCommand, status: NtStatus) -> SvcMessage {
    match command {
        PrinterCommand::Create { request, .. } => create_response(request, 0, status),
        PrinterCommand::Write(request) => write_response(request, 0, status),
        PrinterCommand::Close(request) => close_response(request, status),
    }
}

fn create_response(request: DeviceCreateRequest, file_id: u32, status: NtStatus) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
        device_io_reply: DeviceIoResponse::new(request.device_io_request, status),
        file_id: if status == NtStatus::SUCCESS { file_id } else { 0 },
        information: if status == NtStatus::SUCCESS {
            Information::FILE_OPENED
        } else {
            Information::empty()
        },
    }))
}

fn write_response(request: DeviceWriteRequest, length: u32, status: NtStatus) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
        device_io_reply: DeviceIoResponse::new(request.device_io_request, status),
        length: if status == NtStatus::SUCCESS { length } else { 0 },
    }))
}

fn close_response(request: ironrdp_rdpdr::pdu::efs::DeviceCloseRequest, status: NtStatus) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: DeviceIoResponse::new(request.device_io_request, status),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::*;
    use ironrdp_rdpdr::pdu::efs::{
        CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceIoRequest, FileAttributes,
        MajorFunction, MinorFunction, SharedAccess,
    };

    #[derive(Debug, Default)]
    struct FakeState {
        events: Vec<String>,
        fail_write: bool,
    }

    struct FakeSpooler(Arc<Mutex<FakeState>>);

    impl PrintSpooler for FakeSpooler {
        fn create(&mut self, file_id: u32, _printer: &RdpdrPrinter) -> Result<(), Box<dyn core::error::Error + Send>> {
            self.0.lock().unwrap().events.push(format!("create:{file_id}"));
            Ok(())
        }

        fn write(&mut self, file_id: u32, data: &[u8]) -> Result<(), Box<dyn core::error::Error + Send>> {
            let mut state = self.0.lock().unwrap();
            state.events.push(format!("write:{file_id}:{}", data.len()));
            if state.fail_write {
                Err(Box::new(std::io::Error::other("injected write failure")))
            } else {
                Ok(())
            }
        }

        fn close(&mut self, file_id: u32) -> Result<(), Box<dyn core::error::Error + Send>> {
            self.0.lock().unwrap().events.push(format!("close:{file_id}"));
            Ok(())
        }

        fn abort(&mut self, file_id: u32) {
            self.0.lock().unwrap().events.push(format!("abort:{file_id}"));
        }

        fn reset(&mut self) {
            self.0.lock().unwrap().events.push("reset".to_owned());
        }
    }

    fn printer() -> RdpdrPrinter {
        RdpdrPrinter::new(0xFFFF_FFFE, "Test Printer".to_owned(), "Test Driver".to_owned())
    }

    fn request(file_id: u32, completion_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
        DeviceIoRequest {
            device_id: 0xFFFF_FFFE,
            file_id,
            completion_id,
            major_function,
            minor_function: MinorFunction::from(0),
        }
    }

    fn create_request(path: &str) -> DeviceCreateRequest {
        DeviceCreateRequest {
            device_io_request: request(0, 1, MajorFunction::Create),
            desired_access: DesiredAccess::empty(),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::empty(),
            path: path.to_owned(),
        }
    }

    fn wait_for_completion(worker: &mut PrinterWorker) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let messages = worker.poll().unwrap();
            if let Some(message) = messages.into_iter().next() {
                return message.encode_unframed_pdu().unwrap();
            }
            assert!(Instant::now() < deadline, "printer worker completion timed out");
            std::thread::yield_now();
        }
    }

    fn status(bytes: &[u8]) -> NtStatus {
        NtStatus::from(u32::from_le_bytes(bytes[12..16].try_into().unwrap()))
    }

    #[test]
    fn worker_completes_create_write_close_in_order() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut worker =
            PrinterWorker::with_spooler(printer(), Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();

        assert!(
            worker
                .handle(PrinterIoRequest::Create(create_request("")))
                .unwrap()
                .is_empty()
        );
        let create = wait_for_completion(&mut worker);
        assert_eq!(status(&create), NtStatus::SUCCESS);
        let file_id = u32::from_le_bytes(create[16..20].try_into().unwrap());

        let write = DeviceWriteRequest {
            device_io_request: request(file_id, 2, MajorFunction::Write),
            offset: 0,
            write_data: b"test".to_vec(),
        };
        assert!(worker.handle(PrinterIoRequest::Write(write)).unwrap().is_empty());
        assert_eq!(status(&wait_for_completion(&mut worker)), NtStatus::SUCCESS);

        let close = DeviceCloseRequest {
            device_io_request: request(file_id, 3, MajorFunction::Close),
        };
        assert!(worker.handle(PrinterIoRequest::Close(close)).unwrap().is_empty());
        assert_eq!(status(&wait_for_completion(&mut worker)), NtStatus::SUCCESS);
        assert_eq!(state.lock().unwrap().events, ["create:1", "write:1:4", "close:1"]);
    }

    #[test]
    fn worker_rejects_nonempty_create_path_without_touching_spooler() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut worker =
            PrinterWorker::with_spooler(printer(), Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();

        let responses = worker
            .handle(PrinterIoRequest::Create(create_request(r"C:\forbidden")))
            .unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(
            status(&responses[0].encode_unframed_pdu().unwrap()),
            NtStatus::INVALID_PARAMETER
        );
        assert!(state.lock().unwrap().events.is_empty());
    }

    #[test]
    fn failed_write_aborts_job_and_rejects_later_close() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut worker =
            PrinterWorker::with_spooler(printer(), Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();
        worker.handle(PrinterIoRequest::Create(create_request(""))).unwrap();
        let create = wait_for_completion(&mut worker);
        let file_id = u32::from_le_bytes(create[16..20].try_into().unwrap());
        state.lock().unwrap().fail_write = true;

        worker
            .handle(PrinterIoRequest::Write(DeviceWriteRequest {
                device_io_request: request(file_id, 2, MajorFunction::Write),
                offset: 0,
                write_data: vec![1],
            }))
            .unwrap();
        assert_eq!(status(&wait_for_completion(&mut worker)), NtStatus::UNSUCCESSFUL);

        worker
            .handle(PrinterIoRequest::Close(DeviceCloseRequest {
                device_io_request: request(file_id, 3, MajorFunction::Close),
            }))
            .unwrap();
        assert_eq!(status(&wait_for_completion(&mut worker)), NtStatus::INVALID_HANDLE);
        assert_eq!(state.lock().unwrap().events, ["create:1", "write:1:1", "abort:1"]);
    }

    #[test]
    fn rejected_write_poisons_job_before_later_close() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut worker =
            PrinterWorker::with_spooler(printer(), Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();
        worker.handle(PrinterIoRequest::Create(create_request(""))).unwrap();
        let create = wait_for_completion(&mut worker);
        let file_id = u32::from_le_bytes(create[16..20].try_into().unwrap());

        let rejected = worker.reject_write(request(file_id, 2, MajorFunction::Write));
        assert_eq!(
            status(&rejected[0].encode_unframed_pdu().unwrap()),
            NtStatus::INVALID_PARAMETER
        );
        worker
            .handle(PrinterIoRequest::Close(DeviceCloseRequest {
                device_io_request: request(file_id, 3, MajorFunction::Close),
            }))
            .unwrap();
        assert_eq!(status(&wait_for_completion(&mut worker)), NtStatus::INVALID_HANDLE);
        assert_eq!(state.lock().unwrap().events, ["create:1", "abort:1"]);
    }

    #[test]
    fn worker_shutdown_aborts_open_jobs_before_returning() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut worker =
            PrinterWorker::with_spooler(printer(), Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();
        worker.handle(PrinterIoRequest::Create(create_request(""))).unwrap();
        assert_eq!(status(&wait_for_completion(&mut worker)), NtStatus::SUCCESS);

        drop(worker);

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if state.lock().unwrap().events == ["create:1", "reset"] {
                break;
            }
            assert!(Instant::now() < deadline, "printer worker shutdown timed out");
            std::thread::yield_now();
        }
    }

    #[test]
    #[ignore = "requires IRONRDP_RDPDR_PRINTER_SMOKE=1 and an authorized default printer"]
    fn authorized_default_printer_smoke() {
        if std::env::var_os("IRONRDP_RDPDR_PRINTER_SMOKE").as_deref() != Some(std::ffi::OsStr::new("1")) {
            return;
        }

        let printer = discover_default_printer(0xFFFF_FFFE)
            .expect("default printer discovery succeeds")
            .expect("an authorized default printer is configured");
        let mut worker = PrinterWorker::new(printer, None).expect("printer worker starts");
        worker
            .handle(PrinterIoRequest::Create(create_request("")))
            .expect("queue empty print job");
        let create = wait_for_completion(&mut worker);
        assert_eq!(status(&create), NtStatus::SUCCESS);
        let file_id = u32::from_le_bytes(create[16..20].try_into().unwrap());

        worker
            .handle(PrinterIoRequest::Close(DeviceCloseRequest {
                device_io_request: request(file_id, 2, MajorFunction::Close),
            }))
            .expect("finish empty print job");
        assert_eq!(status(&wait_for_completion(&mut worker)), NtStatus::SUCCESS);
    }
}
