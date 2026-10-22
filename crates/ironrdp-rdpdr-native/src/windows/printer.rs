use core::ffi::c_void;
use core::fmt;
use core::mem::size_of;
use core::slice;
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::time::{Duration, Instant};

use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    DeviceCloseResponse, DeviceCreateRequest, DeviceCreateResponse, DeviceIoResponse, DeviceWriteRequest,
    DeviceWriteResponse, Information, NtStatus, PrinterIoRequest,
};
use ironrdp_rdpdr::{RdpdrPrinter, RdpdrPrinterCacheStore};
use ironrdp_svc::SvcMessage;
use tracing::{debug, warn};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, GetLastError};
use windows::Win32::Graphics::Printing::{
    AbortPrinter, ClosePrinter, DOC_INFO_1W, EndDocPrinter, EnumPrintersW, GetDefaultPrinterW, GetPrinterW,
    OpenPrinterW, PRINTER_ATTRIBUTE_NETWORK, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_HANDLE,
    PRINTER_INFO_2W, StartDocPrinterW, WritePrinter,
};
use windows::core::{Error, HRESULT, PCWSTR, PWSTR};

const MAX_PRINTER_NAME_UTF16_UNITS: usize = 1_024;
const MAX_PRINTER_METADATA_BYTES: usize = 1024 * 1024;
const MAX_PRINTER_LIST_BYTES: usize = 16 * 1024 * 1024;
const MAX_REDIRECTED_PRINTERS: usize = 64;
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

    pub(super) fn acquire(&self) -> RdpdrWorkerThreadGuardResult {
        (self.acquire)()
    }
}

/// Local Windows printer queues selected for redirection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowsPrinterSelection {
    /// The current user's default queue.
    Default,
    /// Every local queue and printer connection, as mstsc and FreeRDP redirect them.
    All,
    /// The listed queues, in order. A queue that cannot be opened is skipped with a warning.
    Named(Vec<String>),
}

/// Printer redirection settings for [`super::WindowsRdpdrBackendFactory`].
///
/// Each product re-reads the selected queues, so printers added or removed locally take effect on
/// the next connection.
#[derive(Clone, Default)]
pub struct WindowsPrinterRedirection {
    selection: Option<WindowsPrinterSelection>,
    driver_overrides: Vec<(String, String)>,
    default_queue: Option<String>,
    cache_store: Option<Arc<dyn RdpdrPrinterCacheStore>>,
}

impl fmt::Debug for WindowsPrinterRedirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowsPrinterRedirection")
            .field("selection", &self.selection)
            .field("driver_overrides", &self.driver_overrides)
            .field("default_queue", &self.default_queue)
            .field("cache_store", &self.cache_store.is_some())
            .finish()
    }
}

impl WindowsPrinterRedirection {
    /// Redirects no printer.
    #[must_use]
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Redirects the queues in `selection`.
    #[must_use]
    pub fn new(selection: WindowsPrinterSelection) -> Self {
        Self {
            selection: Some(selection),
            ..Self::default()
        }
    }

    /// Returns the selected queues, or `None` when printer redirection is disabled.
    #[must_use]
    pub fn selection(&self) -> Option<&WindowsPrinterSelection> {
        self.selection.as_ref()
    }

    /// Announces `driver_name` for the queue named `queue_name` instead of its local driver.
    ///
    /// Queue names compare case-insensitively, like Windows printer names. The override also
    /// takes precedence over a driver the server cached for the queue.
    #[must_use]
    pub fn with_driver(mut self, queue_name: impl Into<String>, driver_name: impl Into<String>) -> Self {
        let queue_name = queue_name.into();
        self.driver_overrides
            .retain(|(existing, _)| !existing.eq_ignore_ascii_case(&queue_name));
        self.driver_overrides.push((queue_name, driver_name.into()));
        self
    }

    /// Marks `queue_name` as the session default printer.
    ///
    /// Without this, the current Windows default queue is marked when it is redirected.
    #[must_use]
    pub fn with_default_queue(mut self, queue_name: Option<String>) -> Self {
        self.default_queue = queue_name;
        self
    }

    /// Persists server printer settings in `store` and replays them on the next connection.
    #[must_use]
    pub fn with_cache_store(mut self, store: Arc<dyn RdpdrPrinterCacheStore>) -> Self {
        self.cache_store = Some(store);
        self
    }

    pub(super) fn cache_store(&self) -> Option<&Arc<dyn RdpdrPrinterCacheStore>> {
        self.cache_store.as_ref()
    }

    /// Reads the selected local queues.
    ///
    /// `allocate_device_id` returns a free RDPDR device ID for each discovered printer.
    pub(super) fn discover(
        &self,
        mut allocate_device_id: impl FnMut() -> Option<u32>,
    ) -> Result<Vec<RdpdrPrinter>, PrinterDiscoveryError> {
        let Some(selection) = &self.selection else {
            return Ok(Vec::new());
        };

        let windows_default = default_printer_name()?;
        let queues = match selection {
            WindowsPrinterSelection::Default => match &windows_default {
                Some(name) => vec![queue_info(name)?],
                None => Vec::new(),
            },
            WindowsPrinterSelection::All => enumerate_queues()?,
            WindowsPrinterSelection::Named(names) => names
                .iter()
                .filter_map(|name| match queue_info(name) {
                    Ok(queue) => Some(queue),
                    Err(error) => {
                        warn!(%error, printer = %name, "Skipping printer that cannot be redirected");
                        None
                    }
                })
                .collect(),
        };

        let default_queue = self.default_queue.as_ref().or(windows_default.as_ref());
        let mut printers = Vec::with_capacity(queues.len());
        for queue in queues {
            if printers.len() == MAX_REDIRECTED_PRINTERS {
                warn!(
                    limit = MAX_REDIRECTED_PRINTERS,
                    "Redirecting only the first printers; the queue count exceeds the limit"
                );
                break;
            }
            if printers
                .iter()
                .any(|printer: &RdpdrPrinter| printer.name().eq_ignore_ascii_case(&queue.name))
            {
                continue;
            }

            let override_driver = self
                .driver_overrides
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&queue.name))
                .map(|(_, driver)| driver.clone());
            let explicit_driver = override_driver.is_some();
            let Some(driver_name) = override_driver.or(queue.driver_name) else {
                warn!(printer = %queue.name, "Skipping printer without a driver name");
                continue;
            };
            if let Err(error) = validate_printer_name(&queue.name).and_then(|()| validate_printer_name(&driver_name)) {
                warn!(%error, printer = %queue.name, "Skipping printer that cannot be redirected");
                continue;
            }
            let device_id = allocate_device_id().ok_or(PrinterDiscoveryError::NoDeviceId)?;
            let is_default = default_queue.is_some_and(|name| name.eq_ignore_ascii_case(&queue.name));
            printers.push(
                RdpdrPrinter::new(device_id, queue.name, driver_name)
                    .with_network(queue.network)
                    .with_default(is_default)
                    .with_explicit_driver(explicit_driver),
            );
        }
        Ok(printers)
    }
}

struct QueueInfo {
    name: String,
    driver_name: Option<String>,
    network: bool,
}

#[derive(Debug)]
pub(super) enum PrinterDiscoveryError {
    Windows(Error),
    InvalidMetadata(&'static str),
    InvalidUtf16,
    NoDeviceId,
}

impl fmt::Display for PrinterDiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Windows(error) => write!(f, "query Windows printers: {error}"),
            Self::InvalidMetadata(message) => f.write_str(message),
            Self::InvalidUtf16 => f.write_str("printer metadata contains invalid UTF-16"),
            Self::NoDeviceId => f.write_str("no RDPDR device ID is available for a printer"),
        }
    }
}

impl core::error::Error for PrinterDiscoveryError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Windows(error) => Some(error),
            Self::InvalidMetadata(_) | Self::InvalidUtf16 | Self::NoDeviceId => None,
        }
    }
}

impl From<Error> for PrinterDiscoveryError {
    fn from(error: Error) -> Self {
        Self::Windows(error)
    }
}

fn validate_printer_name(value: &str) -> Result<(), PrinterDiscoveryError> {
    let units = value.encode_utf16().count();
    if units == 0 {
        return Err(PrinterDiscoveryError::InvalidMetadata(
            "printer metadata must not be empty",
        ));
    }
    if units > MAX_PRINTER_NAME_UTF16_UNITS {
        return Err(PrinterDiscoveryError::InvalidMetadata(
            "printer metadata exceeds the supported length",
        ));
    }
    if value.contains('\0') {
        return Err(PrinterDiscoveryError::InvalidMetadata(
            "printer metadata must not contain NUL",
        ));
    }
    Ok(())
}

fn default_printer_name() -> Result<Option<String>, PrinterDiscoveryError> {
    let mut length = 0u32;
    // SAFETY: A null output buffer is the documented size-probe form and `length` is writable.
    let result = unsafe { GetDefaultPrinterW(None, &mut length) };
    if result.as_bool() {
        return Err(PrinterDiscoveryError::InvalidMetadata(
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
        .map_err(|_| PrinterDiscoveryError::InvalidMetadata("default printer name length does not fit usize"))?;
    if length == 0 || length > MAX_PRINTER_NAME_UTF16_UNITS + 1 {
        return Err(PrinterDiscoveryError::InvalidMetadata(
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
        .map_err(|_| PrinterDiscoveryError::InvalidMetadata("default printer name length does not fit usize"))?;
    if actual == 0 || actual > buffer.len() || buffer[actual - 1] != 0 || buffer[..actual - 1].contains(&0) {
        return Err(PrinterDiscoveryError::InvalidMetadata(
            "default printer name is not a single NUL-terminated string",
        ));
    }

    String::from_utf16(&buffer[..actual - 1])
        .map(Some)
        .map_err(|_| PrinterDiscoveryError::InvalidUtf16)
}

/// Lists local queues and printer connections with `EnumPrintersW` level 2.
fn enumerate_queues() -> Result<Vec<QueueInfo>, PrinterDiscoveryError> {
    const FLAGS: u32 = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;

    // Printers can be added between the size probe and the read; retry a few times.
    for _ in 0..4 {
        let mut needed = 0u32;
        let mut returned = 0u32;
        // SAFETY: A null buffer is the documented size-probe form and both counters are writable.
        let probe = unsafe { EnumPrintersW(FLAGS, PCWSTR::null(), 2, None, &mut needed, &mut returned) };
        if probe.is_ok() || needed == 0 {
            return Ok(Vec::new());
        }

        let buffer_length = usize::try_from(needed)
            .map_err(|_| PrinterDiscoveryError::InvalidMetadata("printer list length does not fit usize"))?;
        if buffer_length > MAX_PRINTER_LIST_BYTES {
            return Err(PrinterDiscoveryError::InvalidMetadata(
                "printer list exceeds the supported size",
            ));
        }
        let mut storage = vec![0usize; buffer_length.div_ceil(size_of::<usize>())];
        // SAFETY: `storage` is contiguous, writable, and spans at least `buffer_length` bytes.
        let bytes = unsafe { slice::from_raw_parts_mut(storage.as_mut_ptr().cast::<u8>(), buffer_length) };
        // SAFETY: `bytes` is a writable buffer of the probed size and both counters are writable.
        match unsafe { EnumPrintersW(FLAGS, PCWSTR::null(), 2, Some(bytes), &mut needed, &mut returned) } {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) => continue,
            Err(error) => return Err(error.into()),
        }

        let count = usize::try_from(returned)
            .map_err(|_| PrinterDiscoveryError::InvalidMetadata("printer count does not fit usize"))?;
        if count
            .checked_mul(size_of::<PRINTER_INFO_2W>())
            .is_none_or(|size| size > buffer_length)
        {
            return Err(PrinterDiscoveryError::InvalidMetadata(
                "printer list count exceeds the returned buffer",
            ));
        }
        // SAFETY: EnumPrintersW level 2 initialized `count` aligned PRINTER_INFO_2W entries at the
        // start of the buffer, and their strings stay valid while `storage` is alive.
        let entries = unsafe { slice::from_raw_parts(storage.as_ptr().cast::<PRINTER_INFO_2W>(), count) };
        let mut queues = Vec::with_capacity(count);
        for entry in entries {
            match queue_from_info(entry) {
                Ok(queue) => queues.push(queue),
                Err(error) => warn!(%error, "Skipping printer with invalid metadata"),
            }
        }
        return Ok(queues);
    }

    Err(PrinterDiscoveryError::InvalidMetadata(
        "printer list kept changing while it was read",
    ))
}

/// Reads one queue's driver and network classification with `GetPrinterW` level 2.
fn queue_info(queue_name: &str) -> Result<QueueInfo, PrinterDiscoveryError> {
    validate_printer_name(queue_name)?;
    let wide_queue_name = wide_nul(queue_name);
    let handle = PrinterHandle::open(&wide_queue_name)?;
    let mut needed = 0u32;
    // SAFETY: `handle` is valid and a null buffer is the documented size-probe form.
    let probe_error = match unsafe { GetPrinterW(handle.raw(), 2, None, &mut needed) } {
        Ok(()) => {
            return Err(PrinterDiscoveryError::InvalidMetadata(
                "printer metadata size probe unexpectedly succeeded",
            ));
        }
        Err(error) => error,
    };
    let buffer_length = usize::try_from(needed)
        .map_err(|_| PrinterDiscoveryError::InvalidMetadata("printer metadata length does not fit usize"))?;
    if buffer_length < size_of::<PRINTER_INFO_2W>() || buffer_length > MAX_PRINTER_METADATA_BYTES {
        return Err(if buffer_length == 0 {
            PrinterDiscoveryError::Windows(probe_error)
        } else {
            PrinterDiscoveryError::InvalidMetadata("printer metadata has an invalid length")
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
    let mut queue = queue_from_info(info)?;
    // Keep the name the caller selected; it opened the queue.
    queue.name = queue_name.to_owned();
    Ok(queue)
}

fn queue_from_info(info: &PRINTER_INFO_2W) -> Result<QueueInfo, PrinterDiscoveryError> {
    if info.pPrinterName.is_null() {
        return Err(PrinterDiscoveryError::InvalidMetadata("printer has no queue name"));
    }
    // SAFETY: The spooler returned a NUL-terminated queue-name pointer valid while its buffer is alive.
    let name = unsafe { info.pPrinterName.to_string() }.map_err(|_| PrinterDiscoveryError::InvalidUtf16)?;
    let driver_name = if info.pDriverName.is_null() {
        None
    } else {
        // SAFETY: The spooler returned a NUL-terminated driver-name pointer valid while its buffer is alive.
        Some(unsafe { info.pDriverName.to_string() }.map_err(|_| PrinterDiscoveryError::InvalidUtf16)?)
    }
    .filter(|driver_name| !driver_name.is_empty());
    Ok(QueueInfo {
        name,
        driver_name,
        network: info.Attributes & PRINTER_ATTRIBUTE_NETWORK != 0,
    })
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
        // The server renders each job with its own copy of the announced driver and sends the
        // result as a PRN file ([MS-RDPEPC 3.2.5.1.2]), which is already in the printer's language.
        // RAW makes the print processor pass it to the port unchanged. The queue's default datatype
        // can be `NT EMF 1.008` or `XPS_PASS`, which would make the print processor try to replay
        // device-ready bytes as EMF or XPS. XPS mode would need `XPS_PASS`, but this client never
        // announces XPSFORMAT.
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
    printers: Vec<RdpdrPrinter>,
    commands: Option<SyncSender<PrinterCommand>>,
    completions: Receiver<SvcMessage>,
    next_file_id: u32,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    poisoned_file_ids: std::sync::Arc<std::sync::Mutex<HashSet<u32>>>,
    // File ID -> device ID of the printer that opened it.
    live_file_ids: std::sync::Arc<std::sync::Mutex<HashMap<u32, u32>>>,
    finished: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    completion_disconnected: bool,
    hooks: Option<RdpdrWorkerThreadHooks>,
    guarded: bool,
}

impl PrinterWorker {
    pub(super) fn new(printers: Vec<RdpdrPrinter>, hooks: Option<RdpdrWorkerThreadHooks>) -> PduResult<Self> {
        Self::with_spooler(printers, Box::<Win32PrintSpooler>::default(), hooks)
    }

    fn with_spooler(
        printers: Vec<RdpdrPrinter>,
        spooler: Box<dyn PrintSpooler>,
        hooks: Option<RdpdrWorkerThreadHooks>,
    ) -> PduResult<Self> {
        let (commands, command_rx) = sync_channel(PRINTER_COMMAND_QUEUE_CAPACITY);
        let (completion_tx, completions) = std::sync::mpsc::channel();
        let worker_printers = printers.clone();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancelled = std::sync::Arc::clone(&cancelled);
        let poisoned_file_ids = std::sync::Arc::new(std::sync::Mutex::new(HashSet::new()));
        let worker_poisoned_file_ids = std::sync::Arc::clone(&poisoned_file_ids);
        let live_file_ids = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
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
                        worker_printers,
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
            printers,
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
                    PrinterCommand::Write(request) => self.poison(&request.device_io_request),
                    PrinterCommand::Close(request) => self.poison(&request.device_io_request),
                    PrinterCommand::Create { .. } => {}
                }
                Ok(vec![failed_command_response(command, NtStatus::UNSUCCESSFUL)])
            }
            Err(TrySendError::Disconnected(_)) => Err(pdu_other_err!("Windows printer worker stopped unexpectedly")),
        }
    }

    pub(super) fn poll(&mut self) -> PduResult<Vec<SvcMessage>> {
        if self.completion_disconnected {
            return Err(pdu_other_err!("Windows printer worker completion channel disconnected"));
        }
        let mut messages = Vec::new();
        loop {
            match self.completions.try_recv() {
                Ok(message) => messages.push(message),
                Err(TryRecvError::Empty) => return Ok(messages),
                Err(TryRecvError::Disconnected) => {
                    warn!("Windows printer worker completion channel disconnected");
                    self.request_stop();
                    if messages.is_empty() {
                        return Err(pdu_other_err!("Windows printer worker completion channel disconnected"));
                    }
                    self.completion_disconnected = true;
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
        let replacement = Self::new(self.printers.clone(), self.hooks)?;
        *self = replacement;
        Ok(())
    }

    pub(super) fn reject_write(&mut self, request: ironrdp_rdpdr::pdu::efs::DeviceIoRequest) -> Vec<SvcMessage> {
        self.poison(&request);
        vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
            device_io_reply: DeviceIoResponse::new(request, NtStatus::INVALID_PARAMETER),
            length: 0,
        }))]
    }

    /// Marks the job of `request` for abort, if the request's printer opened it.
    fn poison(&self, request: &ironrdp_rdpdr::pdu::efs::DeviceIoRequest) {
        if self
            .live_file_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&request.file_id)
            != Some(&request.device_id)
        {
            return;
        }
        self.poisoned_file_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(request.file_id);
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
    printers: Vec<RdpdrPrinter>,
    mut spooler: Box<dyn PrintSpooler>,
    commands: Receiver<PrinterCommand>,
    completions: std::sync::mpsc::Sender<SvcMessage>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    poisoned_file_ids: std::sync::Arc<std::sync::Mutex<HashSet<u32>>>,
    // File ID -> device ID of the printer that opened it.
    live_file_ids: std::sync::Arc<std::sync::Mutex<HashMap<u32, u32>>>,
) {
    // File ID -> (device ID, bytes written). File IDs are unique across this worker's printers.
    let mut jobs = HashMap::<u32, (u32, usize)>::new();
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
            if jobs.remove(&file_id).is_some() {
                spooler.abort(file_id);
                live_file_ids
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&file_id);
            }
        }
        let response = match command {
            PrinterCommand::Create { request, file_id } => {
                let device_id = request.device_io_request.device_id;
                let printer = printers.iter().find(|printer| printer.device_id() == device_id);
                let result = match printer {
                    None => Err("print job targets an unconfigured printer".to_owned()),
                    Some(_) if jobs.len() >= MAX_OPEN_PRINT_JOBS => {
                        Err("maximum open print-job count reached".to_owned())
                    }
                    Some(printer) => spooler.create(file_id, printer).map_err(|error| error.to_string()),
                };
                let status = match result {
                    Ok(()) => {
                        jobs.insert(file_id, (device_id, 0));
                        live_file_ids
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(file_id, device_id);
                        debug!(file_id, device_id, "Started redirected Windows print job");
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
                let device_id = request.device_io_request.device_id;
                let current_size = jobs
                    .get(&file_id)
                    .filter(|(job_device_id, _)| *job_device_id == device_id)
                    .map(|(_, size)| *size);
                let projected_size = current_size
                    .and_then(|current| current.checked_add(request.write_data.len()))
                    .filter(|size| *size <= MAX_PRINT_JOB_BYTES);
                let status = match projected_size {
                    Some(size) => match spooler.write(file_id, &request.write_data) {
                        Ok(()) => {
                            jobs.insert(file_id, (device_id, size));
                            NtStatus::SUCCESS
                        }
                        Err(error) => {
                            warn!(file_id, %error, "Failed to write redirected Windows print data");
                            spooler.abort(file_id);
                            jobs.remove(&file_id);
                            live_file_ids
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .remove(&file_id);
                            NtStatus::UNSUCCESSFUL
                        }
                    },
                    None if current_size.is_none() => {
                        warn!(file_id, device_id, "Rejecting print data for an unknown job");
                        NtStatus::INVALID_HANDLE
                    }
                    None => {
                        warn!(
                            file_id,
                            limit = MAX_PRINT_JOB_BYTES,
                            "Redirected print job exceeds byte limit"
                        );
                        spooler.abort(file_id);
                        jobs.remove(&file_id);
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
                let device_id = request.device_io_request.device_id;
                let status = if jobs
                    .get(&file_id)
                    .is_some_and(|(job_device_id, _)| *job_device_id == device_id)
                {
                    jobs.remove(&file_id);
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
            PrinterWorker::with_spooler(vec![printer()], Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();

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
            PrinterWorker::with_spooler(vec![printer()], Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();

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
            PrinterWorker::with_spooler(vec![printer()], Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();
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
            PrinterWorker::with_spooler(vec![printer()], Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();
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
            PrinterWorker::with_spooler(vec![printer()], Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();
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
    fn disconnected_worker_propagates_an_error() {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let mut worker =
            PrinterWorker::with_spooler(vec![printer()], Box::new(FakeSpooler(Arc::clone(&state))), None).unwrap();
        worker.request_stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.finished.load(std::sync::atomic::Ordering::Acquire) {
            assert!(Instant::now() < deadline, "printer worker shutdown timed out");
            std::thread::yield_now();
        }

        assert!(worker.poll().is_err());
    }

    #[test]
    fn worker_routes_jobs_to_the_printer_of_the_create_request() {
        #[derive(Default)]
        struct QueueRecorder(Arc<Mutex<Vec<String>>>);

        impl PrintSpooler for QueueRecorder {
            fn create(&mut self, _file_id: u32, printer: &RdpdrPrinter) -> SpoolerResult {
                self.0.lock().unwrap().push(printer.name().to_owned());
                Ok(())
            }

            fn write(&mut self, _file_id: u32, _data: &[u8]) -> SpoolerResult {
                Ok(())
            }

            fn close(&mut self, _file_id: u32) -> SpoolerResult {
                Ok(())
            }

            fn abort(&mut self, _file_id: u32) {}

            fn reset(&mut self) {}
        }

        let queues = Arc::new(Mutex::new(Vec::new()));
        let printers = vec![
            RdpdrPrinter::new(7, "First".to_owned(), "Driver".to_owned()),
            RdpdrPrinter::new(8, "Second".to_owned(), "Driver".to_owned()),
        ];
        let mut worker =
            PrinterWorker::with_spooler(printers, Box::new(QueueRecorder(Arc::clone(&queues))), None).unwrap();

        let mut create = create_request("");
        create.device_io_request.device_id = 8;
        worker.handle(PrinterIoRequest::Create(create)).unwrap();
        let response = wait_for_completion(&mut worker);
        assert_eq!(status(&response), NtStatus::SUCCESS);
        let file_id = u32::from_le_bytes(response[16..20].try_into().unwrap());

        // A file ID is bound to the printer that opened it.
        let mut write = DeviceWriteRequest {
            device_io_request: request(file_id, 2, MajorFunction::Write),
            offset: 0,
            write_data: vec![1],
        };
        write.device_io_request.device_id = 7;
        worker.handle(PrinterIoRequest::Write(write)).unwrap();
        assert_eq!(status(&wait_for_completion(&mut worker)), NtStatus::INVALID_HANDLE);

        // An oversized write from the other printer must not abort this job either.
        let mut rejected = request(file_id, 3, MajorFunction::Write);
        rejected.device_id = 7;
        worker.reject_write(rejected);
        let mut close = DeviceCloseRequest {
            device_io_request: request(file_id, 4, MajorFunction::Close),
        };
        close.device_io_request.device_id = 8;
        worker.handle(PrinterIoRequest::Close(close)).unwrap();
        assert_eq!(status(&wait_for_completion(&mut worker)), NtStatus::SUCCESS);
        assert_eq!(*queues.lock().unwrap(), ["Second"]);
    }

    #[test]
    fn all_printer_discovery_reads_local_queues() {
        let mut next_device_id = 100;
        let printers = WindowsPrinterRedirection::new(WindowsPrinterSelection::All)
            .discover(|| {
                next_device_id += 1;
                Some(next_device_id)
            })
            .expect("local printer enumeration succeeds");

        assert!(printers.iter().filter(|printer| printer.is_default()).count() <= 1);
        for printer in &printers {
            assert!(!printer.name().is_empty());
            assert!(!printer.driver_name().is_empty());
            assert!(!printer.explicit_driver());
        }
    }

    #[test]
    fn named_printer_discovery_skips_missing_queues() {
        let printers = WindowsPrinterRedirection::new(WindowsPrinterSelection::Named(vec![
            "IronRDP missing printer 6f0d2c".to_owned(),
        ]))
        .discover(|| Some(1))
        .expect("missing named queues are skipped");

        assert!(printers.is_empty());
    }

    #[test]
    #[ignore = "requires IRONRDP_RDPDR_PRINTER_SMOKE=1 and an authorized default printer"]
    fn authorized_default_printer_smoke() {
        if std::env::var_os("IRONRDP_RDPDR_PRINTER_SMOKE").as_deref() != Some(std::ffi::OsStr::new("1")) {
            return;
        }

        let printers = WindowsPrinterRedirection::new(WindowsPrinterSelection::Default)
            .discover(|| Some(0xFFFF_FFFE))
            .expect("default printer discovery succeeds");
        assert_eq!(printers.len(), 1, "an authorized default printer is configured");
        let mut worker = PrinterWorker::new(printers, None).expect("printer worker starts");
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
