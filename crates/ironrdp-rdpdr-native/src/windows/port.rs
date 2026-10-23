//! Windows serial (COM) and parallel (LPT) port redirection for [MS-RDPESP].
//!
//! Like mstsc, every server create opens its own local handle, with the server's access and
//! share mode and asynchronous native I/O, so a second open of an exclusive port fails as it
//! would locally. A newly opened serial port gets its default configuration and mstsc's read
//! and write timeouts, which bound reads that would otherwise wait for data indefinitely.
//! Reads, writes and device-control requests are forwarded unchanged to the local driver,
//! and the driver's NTSTATUS and transfer count are returned as-is, as MS-RDPESP Appendix A
//! describes for Windows. Requests the driver pends, such as reads waiting for data or
//! `IOCTL_SERIAL_WAIT_ON_MASK`, complete later through [`PortSession::poll`]. Closing a file
//! cancels the requests still pending on it; their completions, then the close completion,
//! follow through [`PortSession::poll`] without blocking the channel. A flush
//! completes once the writes pending on its file when it arrived have completed, which is
//! what the serial driver's own flush waits for.
//!
//! [MS-RDPESP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesp/

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::Instant;

use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{CreateDisposition, DeviceCreateRequest};
use ironrdp_rdpdr::pdu::efs::{
    DeviceCloseResponse, DeviceControlResponse, DeviceCreateResponse, DeviceFlushBuffersResponse, DeviceIoRequest,
    DeviceIoResponse, DeviceReadResponse, DeviceWriteResponse, Information, NtStatus,
};
use ironrdp_rdpdr::pdu::esp::{PortIoCtlCode, PortIoRequest};
use ironrdp_rdpdr::{RdpdrPort, RdpdrPortKind};
use ironrdp_svc::SvcMessage;
use tracing::{debug, warn};
use windows::Wdk::Foundation::OBJECT_ATTRIBUTES;

use super::printer::RdpdrWorkerThreadHooks;
use windows::Wdk::Storage::FileSystem::{
    FILE_NON_DIRECTORY_FILE, NTCREATEFILE_CREATE_DISPOSITION, NtCancelIoFileEx, NtCreateFile, NtReadFile, NtWriteFile,
};
use windows::Wdk::System::IO::NtDeviceIoControlFile;
use windows::Win32::Devices::Communication::{
    COMMCONFIG, COMMTIMEOUTS, GetCommTimeouts, GetDefaultCommConfigW, SetCommState, SetCommTimeouts,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE, UNICODE_STRING, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    FILE_ACCESS_RIGHTS, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_MODE, QueryDosDeviceW,
};
use windows::Win32::System::IO::IO_STATUS_BLOCK;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::{PCWSTR, PWSTR};

/// Largest read or device-control output buffer allocated for one server request.
///
/// MS-RDPESP allows partial reads, so longer reads are shortened. Serial and parallel
/// control structures are far smaller.
const MAX_PORT_TRANSFER: usize = 64 * 1024;
const MAX_PENDING_PORT_OPERATIONS: usize = 64;
const MAX_OPEN_PORT_FILES: usize = 64;
/// Stack reserved for each pending-request worker, which only waits on an event and stores the result.
const WORKER_STACK_SIZE: usize = 64 * 1024;
/// How long the driver has to complete a cancelled request before it is reported as cancelled.
const CANCELLATION_TIMEOUT: Duration = Duration::from_secs(2);
const STATUS_PENDING: NTSTATUS = NTSTATUS(0x0000_0103);

/// Timeouts mstsc sets on every newly opened serial port, in milliseconds.
const READ_INTERVAL_TIMEOUT_MS: u32 = 200;
const READ_TOTAL_TIMEOUT_CONSTANT_MS: u32 = 5_000;
const WRITE_TOTAL_TIMEOUT_CONSTANT_MS: u32 = 60_000;
/// Marks a DOS device that is itself a redirected port, as in a nested remote session.
const REDIRECTED_PORT_TARGET: &str = "RDPDR";

/// Which local ports of one kind to redirect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PortSelection {
    /// Redirect no port of this kind.
    #[default]
    None,
    /// Redirect every port of this kind present when the connection starts.
    All,
    /// Redirect the named ports, such as `COM3` or `LPT1`, that are present when the connection starts.
    ///
    /// Like mstsc, only `COM<n>` serial and `LPT<n>` parallel ports can be selected, so a name
    /// cannot expose another kind of device to the server.
    Named(Vec<String>),
}

impl PortSelection {
    fn is_none(&self) -> bool {
        match self {
            Self::None => true,
            Self::All => false,
            Self::Named(names) => names.is_empty(),
        }
    }
}

/// Serial and parallel port selection for one connection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowsPortRedirection {
    /// Serial (COM) ports to redirect.
    pub serial: PortSelection,
    /// Parallel (LPT) ports to redirect.
    pub parallel: PortSelection,
}

impl WindowsPortRedirection {
    /// Redirects every local serial and parallel port, like mstsc's `redirectcomports:i:1`.
    #[must_use]
    pub fn all() -> Self {
        Self {
            serial: PortSelection::All,
            parallel: PortSelection::All,
        }
    }

    /// Redirects every port when `enabled` is true, and none otherwise.
    #[must_use]
    pub fn from_enabled(enabled: bool) -> Self {
        if enabled { Self::all() } else { Self::default() }
    }

    /// Returns whether any port may be redirected.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        !self.serial.is_none() || !self.parallel.is_none()
    }
}

/// A serial or parallel port present on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalPort {
    kind: RdpdrPortKind,
    /// The DOS device name, such as `COM3` or `LPT1`.
    name: String,
}

/// Lists the serial and parallel ports on this computer.
///
/// Like mstsc, ports are the `COM<n>` and `LPT<n>` DOS devices, including ports that are
/// currently in use. Ports that are themselves redirected from another client, as in a
/// nested remote session, are skipped. Unlike mstsc, which probes `COM1`..`COM32` and
/// `LPT0`..`LPT9` by opening each one, the DOS device list is read without opening the
/// ports, and there is no upper bound on the port number.
fn local_ports() -> Vec<LocalPort> {
    let dos_devices = dos_device_names();
    let mut ports = Vec::new();
    for (kind, prefix) in [(RdpdrPortKind::Serial, "COM"), (RdpdrPortKind::Parallel, "LPT")] {
        let mut names = dos_devices
            .iter()
            .filter(|name| {
                name.strip_prefix(prefix)
                    .is_some_and(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
            })
            .filter(|name| !is_redirected_port(name))
            .cloned()
            .collect::<Vec<_>>();
        names.sort_by_key(|name| port_sort_key(name));
        names.dedup();
        ports.extend(names.into_iter().map(|name| LocalPort { kind, name }));
    }
    ports
}

/// Resolves a port selection against the local ports, allocating device IDs with `allocate_device_id`.
pub(super) fn resolve_ports(
    redirection: &WindowsPortRedirection,
    mut allocate_device_id: impl FnMut() -> Option<u32>,
) -> Vec<RdpdrPort> {
    if !redirection.is_enabled() {
        return Vec::new();
    }

    let local = local_ports();
    let mut selected = Vec::new();
    for (kind, selection) in [
        (RdpdrPortKind::Serial, &redirection.serial),
        (RdpdrPortKind::Parallel, &redirection.parallel),
    ] {
        match selection {
            PortSelection::None => {}
            PortSelection::All => selected.extend(
                local
                    .iter()
                    .filter(|port| port.kind == kind)
                    .map(|port| (kind, port.name.clone())),
            ),
            PortSelection::Named(names) => {
                for requested in names {
                    let Some(name) = normalize_port_name(requested) else {
                        warn!(port = %requested, "Ignoring invalid port name");
                        continue;
                    };
                    if local.iter().any(|port| port.kind == kind && port.name == name) {
                        selected.push((kind, name));
                    } else {
                        warn!(port = %name, ?kind, "Ignoring a name that is not a local port of this kind");
                    }
                }
            }
        }
    }

    let mut seen = HashSet::new();
    let mut ports = Vec::with_capacity(selected.len());
    for (kind, name) in selected {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(device_id) = allocate_device_id() else {
            warn!(port = %name, "No RDPDR device ID left for port");
            break;
        };
        match RdpdrPort::new(device_id, kind, name.clone()) {
            Ok(port) => ports.push(port),
            Err(error) => warn!(port = %name, %error, "Ignoring port that cannot be announced"),
        }
    }
    ports
}

/// Uppercases a port name and removes an optional `\\.\` prefix and trailing colon.
fn normalize_port_name(name: &str) -> Option<String> {
    let name = name.trim().trim_end_matches('\0');
    let name = name
        .strip_prefix(r"\\.\")
        .or_else(|| name.strip_prefix(r"\DosDevices\"))
        .or_else(|| name.strip_prefix(r"\??\"))
        .unwrap_or(name);
    let name = name.strip_suffix(':').unwrap_or(name).to_ascii_uppercase();
    // RdpdrPort validates the announced form; only reject names that cannot be a DOS device.
    (!name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric())).then_some(name)
}

/// Sorts `COM2` before `COM10`.
fn port_sort_key(name: &str) -> (String, u32) {
    let digits = name.trim_start_matches(|ch: char| !ch.is_ascii_digit());
    let prefix = &name[..name.len() - digits.len()];
    (prefix.to_owned(), digits.parse().unwrap_or(u32::MAX))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(core::iter::once(0)).collect()
}

/// Returns the uppercased names of every DOS device visible to this process.
fn dos_device_names() -> Vec<String> {
    let mut buffer = vec![0u16; 64 * 1024];
    loop {
        // SAFETY: a null device name requests the list of all DOS device names, written as
        // NUL-separated strings into `buffer`, whose length the binding passes alongside it.
        let length = unsafe { QueryDosDeviceW(PCWSTR::null(), Some(&mut buffer)) };
        if length != 0 {
            let length = usize::try_from(length).expect("u32 fits in usize").min(buffer.len());
            return buffer[..length]
                .split(|unit| *unit == 0)
                .filter(|name| !name.is_empty())
                .map(|name| String::from_utf16_lossy(name).to_ascii_uppercase())
                .collect();
        }
        if buffer.len() >= 4 * 1024 * 1024 {
            warn!("Failed to list DOS devices");
            return Vec::new();
        }
        // The list did not fit; QueryDosDeviceW reports ERROR_INSUFFICIENT_BUFFER.
        let grown = buffer.len() * 4;
        buffer.resize(grown, 0);
    }
}

/// Whether a DOS device is itself a redirected port, as in a nested remote session.
fn is_redirected_port(name: &str) -> bool {
    let name = wide(name);
    let mut target = [0u16; 1024];
    // SAFETY: `name` is a NUL-terminated UTF-16 string and `target` is a writable buffer whose
    // length the binding passes alongside it.
    let length = unsafe { QueryDosDeviceW(PCWSTR(name.as_ptr()), Some(&mut target)) };
    let length = usize::try_from(length).expect("u32 fits in usize").min(target.len());
    // The result holds NUL-separated targets; the first one is the active link.
    let first = target[..length].split(|unit| *unit == 0).next().unwrap_or_default();
    String::from_utf16_lossy(first)
        .to_ascii_uppercase()
        .contains(REDIRECTED_PORT_TARGET)
}

/// An owned native handle closed on drop.
#[derive(Debug)]
struct OwnedHandle(HANDLE);

// SAFETY: the wrapper owns the kernel handle and only closes it on drop. Kernel handles may be
// used from any thread, and asynchronous requests on a port handle are serialized by the driver.
unsafe impl Send for OwnedHandle {}
// SAFETY: shared access only reads the handle value; the handle is closed once, after every
// shared reference is released.
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by NtCreateFile or CreateEventW and is closed once here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Returns the NT path of a port's DOS device, such as `\??\COM3`.
fn port_device_path(name: &str) -> String {
    format!(r"\??\{name}")
}

/// Opens a port for one server create with the server's access and share mode.
///
/// As mstsc does, FILE_SHARE_DELETE is dropped and the supersede and overwrite dispositions,
/// which have no meaning for a device, are rejected.
fn open_port(path: &str, create: &DeviceCreateRequest) -> Result<OwnedHandle, NtStatus> {
    if matches!(
        create.create_disposition,
        CreateDisposition::FILE_SUPERSEDE | CreateDisposition::FILE_OVERWRITE
    ) {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let share_access = FILE_SHARE_MODE(create.shared_access.bits()) & !FILE_SHARE_DELETE;

    let mut path = path.encode_utf16().collect::<Vec<_>>();
    let path_length = path
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or(NtStatus::OBJECT_NAME_INVALID)?;
    let mut unicode_name = UNICODE_STRING {
        Length: path_length,
        MaximumLength: path_length,
        Buffer: PWSTR::from_raw(path.as_mut_ptr()),
    };
    let object_attributes = OBJECT_ATTRIBUTES {
        Length: u32::try_from(size_of::<OBJECT_ATTRIBUTES>()).expect("OBJECT_ATTRIBUTES size fits in u32"),
        RootDirectory: HANDLE::default(),
        ObjectName: &mut unicode_name,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: core::ptr::null(),
        SecurityQualityOfService: core::ptr::null(),
    };
    let mut handle = HANDLE::default();
    let mut io_status = IO_STATUS_BLOCK::default();

    // SAFETY:
    // - `path`, `unicode_name` and `object_attributes` stay alive for the entire call.
    // - `handle` and `io_status` are valid writable storage.
    // - without FILE_SYNCHRONOUS_IO_* the handle is opened for asynchronous I/O.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_ACCESS_RIGHTS(create.desired_access.bits()),
            &object_attributes,
            &mut io_status,
            None,
            FILE_ATTRIBUTE_NORMAL,
            share_access,
            NTCREATEFILE_CREATE_DISPOSITION(u32::from(create.create_disposition)),
            FILE_NON_DIRECTORY_FILE,
            None,
            0,
        )
    };
    if status.is_err() {
        return Err(to_nt_status(status));
    }

    Ok(OwnedHandle(handle))
}

/// Gives a newly opened serial port its default configuration and mstsc's timeouts.
///
/// Like mstsc, nothing is changed when the default configuration cannot be read, and errors
/// are ignored: the server configures the port itself through IOCTLs.
fn apply_serial_defaults(handle: HANDLE, name: &str) {
    let name = wide(name);
    // COMMCONFIG ends with provider-specific data, so leave room after the fixed part.
    let mut buffer = vec![0u64; 128];
    let mut size = u32::try_from(size_of_val(buffer.as_slice())).expect("buffer size fits in u32");
    let config = buffer.as_mut_ptr().cast::<COMMCONFIG>();
    // SAFETY: `name` is NUL-terminated, and `config` points to `size` writable bytes aligned
    // for COMMCONFIG.
    if let Err(error) = unsafe { GetDefaultCommConfigW(PCWSTR(name.as_ptr()), config, &mut size) } {
        debug!(%error, "No default serial port configuration; keeping the current one");
        return;
    }
    // SAFETY: GetDefaultCommConfigW initialized the configuration.
    let dcb = unsafe { (*config).dcb };
    // SAFETY: the handle is an open serial port and `dcb` is a valid DCB.
    if let Err(error) = unsafe { SetCommState(handle, &dcb) } {
        debug!(%error, "Failed to apply the default serial port configuration");
    }

    let mut timeouts = COMMTIMEOUTS::default();
    // SAFETY: the handle is an open serial port and `timeouts` is writable.
    if let Err(error) = unsafe { GetCommTimeouts(handle, &mut timeouts) } {
        debug!(%error, "Failed to read serial port timeouts");
        return;
    }
    timeouts.ReadIntervalTimeout = READ_INTERVAL_TIMEOUT_MS;
    timeouts.ReadTotalTimeoutConstant = READ_TOTAL_TIMEOUT_CONSTANT_MS;
    timeouts.WriteTotalTimeoutConstant = WRITE_TOTAL_TIMEOUT_CONSTANT_MS;
    // SAFETY: the handle is an open serial port and `timeouts` is a valid COMMTIMEOUTS.
    if let Err(error) = unsafe { SetCommTimeouts(handle, &timeouts) } {
        debug!(%error, "Failed to set serial port timeouts");
    }
}

fn to_nt_status(status: NTSTATUS) -> NtStatus {
    NtStatus::from(u32::from_ne_bytes(status.0.to_ne_bytes()))
}

/// Native state of one asynchronous request. It lives on the heap so that the I/O status
/// block and buffers keep their address until the driver completes the request.
struct PortIo {
    kind: PortIoKind,
    request: DeviceIoRequest,
    event: OwnedHandle,
    io_status: IO_STATUS_BLOCK,
    input: Vec<u8>,
    output: Vec<u8>,
}

// SAFETY: the request is owned by exactly one thread at a time: the processor thread that
// submits it, then the worker that waits for it. The kernel writes the status block and
// output buffer only until it signals the event, which the owner waits for before reading.
unsafe impl Send for PortIo {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PortIoKind {
    Read,
    Write,
    DeviceControl(PortIoCtlCode),
}

/// Like mstsc, a wait-on-mask completion always carries its 4-byte event mask.
fn control_output(code: PortIoCtlCode, mut output: Vec<u8>) -> Vec<u8> {
    if code == PortIoCtlCode::IOCTL_SERIAL_WAIT_ON_MASK {
        output.resize(size_of::<u32>(), 0);
    }
    output
}

impl PortIo {
    fn new(
        kind: PortIoKind,
        request: DeviceIoRequest,
        input: Vec<u8>,
        output_length: usize,
    ) -> Result<Box<Self>, NtStatus> {
        // SAFETY: CreateEventW has no pointer arguments here; it creates an unnamed
        // manual-reset event that the returned handle owns.
        let event =
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|_| NtStatus::INSUFFICIENT_RESOURCES)?;
        Ok(Box::new(Self {
            kind,
            request,
            event: OwnedHandle(event),
            io_status: IO_STATUS_BLOCK::default(),
            input,
            output: vec![0; output_length],
        }))
    }

    /// Submits the request and returns the immediate status, `STATUS_PENDING` if the driver queued it.
    ///
    /// # Safety
    ///
    /// When this returns `STATUS_PENDING`, `self` must stay allocated and unmoved until its event is signaled.
    unsafe fn submit(&mut self, handle: HANDLE) -> NTSTATUS {
        // MS-RDPESP: the Offset field of reads and writes is ignored. Asynchronous handles
        // still require an explicit byte offset.
        let offset = 0i64;
        let input_length = u32::try_from(self.input.len()).expect("PDU-bounded input fits in u32");
        let output_length = u32::try_from(self.output.len()).expect("MAX_PORT_TRANSFER fits in u32");
        match self.kind {
            // SAFETY: the handle is open, and the event, status block and buffer are owned by this
            // heap allocation, which the caller keeps alive until the request completes.
            PortIoKind::Read => unsafe {
                NtReadFile(
                    handle,
                    Some(self.event.0),
                    None,
                    None,
                    &mut self.io_status,
                    self.output.as_mut_ptr().cast(),
                    output_length,
                    Some(&offset),
                    None,
                )
            },
            // SAFETY: as for reads; the input buffer is not modified while the write is in flight.
            PortIoKind::Write => unsafe {
                NtWriteFile(
                    handle,
                    Some(self.event.0),
                    None,
                    None,
                    &mut self.io_status,
                    self.input.as_ptr().cast(),
                    input_length,
                    Some(&offset),
                    None,
                )
            },
            // SAFETY: as for reads; empty buffers are passed as null pointers with a zero length.
            PortIoKind::DeviceControl(code) => unsafe {
                NtDeviceIoControlFile(
                    handle,
                    Some(self.event.0),
                    None,
                    None,
                    &mut self.io_status,
                    u32::from(code),
                    (!self.input.is_empty()).then(|| self.input.as_ptr().cast()),
                    input_length,
                    (!self.output.is_empty()).then(|| self.output.as_mut_ptr().cast()),
                    output_length,
                )
            },
        }
    }

    /// Waits up to `timeout_ms` for the driver to complete the request, and returns whether it did.
    ///
    /// When this returns false, the caller must leak the request, because the driver may still
    /// write to it.
    fn wait(&self, timeout_ms: u32) -> bool {
        // SAFETY: the event handle is owned by `self` and stays open during the wait.
        let result = unsafe { WaitForSingleObject(self.event.0, timeout_ms) };
        result == WAIT_OBJECT_0
    }

    /// Completes a request this thread still owns after cancelling it.
    ///
    /// The wait is bounded like a cancellation on close. A request the driver does not complete
    /// in time is leaked and reported as cancelled.
    fn complete_cancelled(mut self: Box<Self>) -> SvcMessage {
        let timeout_ms = u32::try_from(CANCELLATION_TIMEOUT.as_millis()).expect("timeout fits in u32");
        if !self.wait(timeout_ms) {
            warn!(
                completion_id = self.request.completion_id,
                "Port driver did not complete a cancelled request; completing it as cancelled"
            );
            let message = failure_completion(self.kind, self.request.clone(), NtStatus::CANCELLED);
            Box::leak(self);
            return message;
        }
        let status = self.final_status();
        self.completion(status)
    }

    /// Builds the completion from the final status of a request that is no longer in flight.
    fn completion(&mut self, status: NTSTATUS) -> SvcMessage {
        let io_status = to_nt_status(status);
        // Errors carry no data; warnings such as STATUS_BUFFER_OVERFLOW still return partial output.
        let transferred = if status.0 < 0 && !is_warning(status) {
            0
        } else {
            self.io_status.Information
        };
        let reply = DeviceIoResponse::new(self.request.clone(), io_status);
        match self.kind {
            PortIoKind::Read => {
                self.output.truncate(transferred.min(self.output.len()));
                SvcMessage::from(RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                    device_io_reply: reply,
                    read_data: core::mem::take(&mut self.output),
                }))
            }
            PortIoKind::Write => SvcMessage::from(RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                device_io_reply: reply,
                length: u32::try_from(transferred.min(self.input.len())).expect("PDU-bounded write fits in u32"),
            })),
            PortIoKind::DeviceControl(code) => {
                self.output.truncate(transferred.min(self.output.len()));
                SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse::with_raw_output(
                    self.request.clone(),
                    io_status,
                    control_output(code, core::mem::take(&mut self.output)),
                )))
            }
        }
    }

    /// Returns the final status once the event has been signaled.
    fn final_status(&self) -> NTSTATUS {
        // SAFETY: the driver completed the request, so the status union holds its final NTSTATUS.
        unsafe { self.io_status.Anonymous.Status }
    }
}

fn is_warning(status: NTSTATUS) -> bool {
    // NTSTATUS severity 2 (warning) has the top two bits set to 10.
    u32::from_ne_bytes(status.0.to_ne_bytes()) >> 30 == 0b10
}

#[derive(Debug)]
struct PendingPortIo {
    device_id: u32,
    file_id: u32,
    kind: PortIoKind,
    request: DeviceIoRequest,
    handle: Arc<OwnedHandle>,
    /// Address of the request's I/O status block, the key NtCancelIoFileEx matches.
    ///
    /// INVARIANT: the request stays allocated while this entry exists, so the address cannot
    /// belong to another request on the shared handle.
    io_status_block: usize,
    slot: Arc<PortIoSlot>,
    /// When the request was cancelled, the time by which the driver must have completed it.
    cancel_deadline: Option<Instant>,
}

/// Where a worker hands a completed request back to the session.
///
/// The worker does not build the completion itself: the request stays in the slot until the
/// session removes its pending entry, which keeps its status-block address from being reused
/// while it may still be passed to NtCancelIoFileEx. Workers may also exit through a host
/// lifetime guard, which never returns a value from the thread.
#[derive(Default)]
struct PortIoSlot {
    io: Mutex<Option<Box<PortIo>>>,
    finished: AtomicBool,
}

impl PortIoSlot {
    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Builds the completion of a finished request.
    fn completion(&self, kind: PortIoKind, request: DeviceIoRequest) -> SvcMessage {
        let io = self.io.lock().unwrap_or_else(PoisonError::into_inner).take();
        match io {
            Some(mut io) => {
                let status = io.final_status();
                io.completion(status)
            }
            None => {
                tracing::error!(completion_id = request.completion_id, "Port worker lost its request");
                failure_completion(kind, request, NtStatus::UNSUCCESSFUL)
            }
        }
    }
}

impl fmt::Debug for PortIoSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PortIoSlot")
            .field("finished", &self.is_finished())
            .finish_non_exhaustive()
    }
}

/// A flush waits for the writes on its file that were pending when it arrived.
#[derive(Debug)]
struct PendingFlush {
    file_id: u32,
    request: DeviceIoRequest,
    writes: Vec<u64>,
}

/// Serial and parallel port state for one RDPDR channel lifetime.
#[derive(Debug)]
pub(super) struct PortSession {
    ports: HashMap<u32, ConfiguredPort>,
    files: HashMap<u32, OpenPortFile>,
    next_file_id: u32,
    next_operation_id: u64,
    pending: HashMap<u64, PendingPortIo>,
    flushes: Vec<PendingFlush>,
    /// Closes waiting for the cancelled requests of their file to complete.
    closes: Vec<DeviceIoRequest>,
    /// Requests whose cancellation the driver did not complete in time.
    abandoned: Vec<Arc<PortIoSlot>>,
    worker_thread_hooks: Option<RdpdrWorkerThreadHooks>,
}

#[derive(Debug)]
struct ConfiguredPort {
    kind: RdpdrPortKind,
    /// The DOS name, such as `COM3`.
    name: String,
    /// The NT path opened for each server create, such as `\??\COM3`.
    path: String,
}

/// One server file, backed by its own local handle.
#[derive(Debug)]
struct OpenPortFile {
    device_id: u32,
    handle: Arc<OwnedHandle>,
}

impl PortSession {
    pub(super) fn new(ports: Vec<RdpdrPort>, worker_thread_hooks: Option<RdpdrWorkerThreadHooks>) -> Self {
        Self {
            ports: ports
                .into_iter()
                .map(|port| {
                    (
                        port.device_id(),
                        ConfiguredPort {
                            kind: port.kind(),
                            name: port.name().to_owned(),
                            path: port_device_path(port.name()),
                        },
                    )
                })
                .collect(),
            files: HashMap::new(),
            next_file_id: 1,
            next_operation_id: 1,
            pending: HashMap::new(),
            flushes: Vec::new(),
            closes: Vec::new(),
            abandoned: Vec::new(),
            worker_thread_hooks,
        }
    }

    /// Requests that hold resources until the driver completes them.
    fn outstanding(&self) -> usize {
        self.pending.len() + self.flushes.len()
    }

    pub(super) fn handle(&mut self, req: PortIoRequest) -> Vec<SvcMessage> {
        let device_id = req.device_io_request().device_id;
        if !self.ports.contains_key(&device_id) {
            return vec![SvcMessage::from(req.failure_response(NtStatus::INVALID_DEVICE_REQUEST))];
        }

        if !matches!(req, PortIoRequest::Create(_)) {
            let file_id = req.device_io_request().file_id;
            if self.files.get(&file_id).map(|file| file.device_id) != Some(device_id) {
                return vec![SvcMessage::from(req.failure_response(NtStatus::INVALID_HANDLE))];
            }
        }

        match req {
            PortIoRequest::Create(create) => {
                vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(self.create(create)))]
            }
            PortIoRequest::Close(close) => self.close(close.device_io_request),
            PortIoRequest::Read(read) => {
                let length = usize::try_from(read.length)
                    .unwrap_or(usize::MAX)
                    .min(MAX_PORT_TRANSFER);
                self.submit(PortIoKind::Read, read.device_io_request, Vec::new(), length)
            }
            PortIoRequest::Write(write) => self.submit(PortIoKind::Write, write.device_io_request, write.write_data, 0),
            PortIoRequest::DeviceControl(control) => {
                let output_length = usize::try_from(control.request.output_buffer_length)
                    .unwrap_or(usize::MAX)
                    .min(MAX_PORT_TRANSFER);
                let code = control.request.io_control_code;
                // Rejected before reaching the driver, so there is no output, not even a wait mask.
                if let Err(status) = code.check_buffers(&control.input_buffer, control.request.output_buffer_length) {
                    return vec![SvcMessage::from(RdpdrPdu::DeviceControlResponse(
                        DeviceControlResponse::with_raw_output(control.request.header, status, Vec::new()),
                    ))];
                }
                self.submit(
                    PortIoKind::DeviceControl(code),
                    control.request.header,
                    control.input_buffer,
                    output_length,
                )
            }
            PortIoRequest::FlushBuffers(flush) => self.flush(flush.device_io_request),
        }
    }

    fn create(&mut self, create: DeviceCreateRequest) -> DeviceCreateResponse {
        let request = create.device_io_request.clone();
        let device_id = request.device_id;
        let fail = |request, status| DeviceCreateResponse {
            device_io_reply: DeviceIoResponse::new(request, status),
            file_id: 0,
            information: Information::empty(),
        };
        if self.files.len() == MAX_OPEN_PORT_FILES {
            return fail(request, NtStatus::INSUFFICIENT_RESOURCES);
        }
        let port = self.ports.get(&device_id).expect("the caller checked the device ID");

        let handle = match open_port(&port.path, &create) {
            Ok(handle) => handle,
            Err(status) => {
                debug!(port = %port.name, device_id, ?status, "Failed to open redirected port");
                return fail(request, status);
            }
        };
        if port.kind == RdpdrPortKind::Serial {
            apply_serial_defaults(handle.0, &port.name);
        }
        debug!(port = %port.name, device_id, "Opened redirected port");

        let file_id = self.allocate_file_id();
        self.files.insert(
            file_id,
            OpenPortFile {
                device_id,
                handle: Arc::new(handle),
            },
        );
        DeviceCreateResponse {
            device_io_reply: DeviceIoResponse::new(request, NtStatus::SUCCESS),
            file_id,
            // MS-RDPESP 3.2.5.1.7: the Information field MUST be set to 0.
            information: Information::empty(),
        }
    }

    fn allocate_file_id(&mut self) -> u32 {
        loop {
            let file_id = self.next_file_id;
            self.next_file_id = self.next_file_id.checked_add(1).unwrap_or(1);
            // A closing file keeps its ID until its close completes.
            if !self.files.contains_key(&file_id) && !self.closes.iter().any(|close| close.file_id == file_id) {
                return file_id;
            }
        }
    }

    fn close(&mut self, request: DeviceIoRequest) -> Vec<SvcMessage> {
        // MS-RDPESP 3.2.5.1.3: the client MUST cancel pending I/O on the port before closing it.
        // The cancelled requests complete through poll, followed by the close itself.
        let deadline = Instant::now() + CANCELLATION_TIMEOUT;
        for io in self.pending.values_mut() {
            if io.file_id == request.file_id && io.cancel_deadline.is_none() {
                cancel_request(&io.handle, io.io_status_block);
                io.cancel_deadline = Some(deadline);
            }
        }
        let mut messages = Vec::new();
        self.flushes.retain(|flush| {
            if flush.file_id == request.file_id {
                messages.push(flush_completion(flush.request.clone(), NtStatus::CANCELLED));
                false
            } else {
                true
            }
        });
        // Requests still finishing keep the handle open until they complete.
        self.files.remove(&request.file_id);
        debug!(
            device_id = request.device_id,
            file_id = request.file_id,
            "Closed redirected port"
        );

        if self.pending.values().any(|io| io.file_id == request.file_id) {
            self.closes.push(request);
        } else {
            messages.push(close_completion(request));
        }
        messages
    }

    fn submit(
        &mut self,
        kind: PortIoKind,
        request: DeviceIoRequest,
        input: Vec<u8>,
        output_length: usize,
    ) -> Vec<SvcMessage> {
        let handle = Arc::clone(
            &self
                .files
                .get(&request.file_id)
                .expect("the caller checked the file ID")
                .handle,
        );
        if self.outstanding() >= MAX_PENDING_PORT_OPERATIONS {
            return vec![failure_completion(kind, request, NtStatus::INSUFFICIENT_RESOURCES)];
        }
        let mut io = match PortIo::new(kind, request.clone(), input, output_length) {
            Ok(io) => io,
            Err(status) => return vec![failure_completion(kind, request, status)],
        };

        // SAFETY: a pending request stays boxed and owned, first by this function and then by
        // the worker that waits for its event, until the driver completes it.
        let status = unsafe { io.submit(handle.0) };
        if status != STATUS_PENDING {
            // A request the driver completed synchronously has already updated its status block.
            // One rejected before reaching the driver carries no data.
            let status = if status.is_ok() || is_warning(status) {
                io.final_status()
            } else {
                status
            };
            return vec![io.completion(status)];
        }

        let io_status_block = core::ptr::addr_of!(io.io_status) as usize;
        // A host such as an ActiveX control keeps its module loaded while the worker runs.
        let guard = match self
            .worker_thread_hooks
            .as_ref()
            .map(RdpdrWorkerThreadHooks::acquire)
            .transpose()
        {
            Ok(guard) => guard,
            Err(error) => {
                warn!(%error, "Failed to acquire a port worker lifetime; cancelling the request");
                cancel_request(&handle, io_status_block);
                return vec![io.complete_cancelled()];
            }
        };
        let slot = Arc::new(PortIoSlot::default());
        let worker_slot = Arc::clone(&slot);
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        // The request is handed over only once the worker exists, so a failed spawn leaves it here.
        let (sender, receiver) = mpsc::sync_channel::<Box<PortIo>>(1);
        let spawned = thread::Builder::new()
            .name("ironrdp-rdpdr-port".to_owned())
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    if let Ok(io) = receiver.recv() {
                        if io.wait(INFINITE) {
                            *worker_slot.io.lock().unwrap_or_else(PoisonError::into_inner) = Some(io);
                        } else {
                            tracing::error!("Failed to wait for a port request");
                            Box::leak(io);
                        }
                    }
                });
                worker_slot.finished.store(true, Ordering::Release);
                // A guarded exit ends the thread without running destructors.
                drop(worker_slot);
                drop(receiver);
                drop(dispatch);
                if let Some(guard) = guard {
                    guard.exit();
                }
            });
        // The thread is never joined: a guarded exit does not return through it.
        if let Err(error) = spawned {
            warn!(%error, "Failed to start a port worker; cancelling the request");
            cancel_request(&handle, io_status_block);
            return vec![io.complete_cancelled()];
        }
        if let Err(mpsc::SendError(io)) = sender.send(io) {
            // The worker exited before receiving the request, so it is still owned here.
            cancel_request(&handle, io_status_block);
            return vec![io.complete_cancelled()];
        }

        let operation_id = self.next_operation_id;
        self.next_operation_id = self.next_operation_id.wrapping_add(1);

        self.pending.insert(
            operation_id,
            PendingPortIo {
                device_id: request.device_id,
                file_id: request.file_id,
                kind,
                request,
                handle,
                io_status_block,
                slot,
                cancel_deadline: None,
            },
        );
        Vec::new()
    }

    /// The handle is asynchronous and NtFlushBuffersFile has no completion event, so the flush is
    /// tracked here instead of being sent to the driver.
    fn flush(&mut self, request: DeviceIoRequest) -> Vec<SvcMessage> {
        let writes = self
            .pending
            .iter()
            .filter_map(|(operation_id, io)| {
                (io.file_id == request.file_id && io.kind == PortIoKind::Write).then_some(*operation_id)
            })
            .collect::<Vec<_>>();
        if writes.is_empty() {
            return vec![flush_completion(request, NtStatus::SUCCESS)];
        }
        if self.outstanding() >= MAX_PENDING_PORT_OPERATIONS {
            return vec![flush_completion(request, NtStatus::INSUFFICIENT_RESOURCES)];
        }

        self.flushes.push(PendingFlush {
            file_id: request.file_id,
            request,
            writes,
        });
        Vec::new()
    }

    /// Returns the completions of requests the driver finished since the last poll, and of the
    /// closes whose cancelled requests have all completed.
    pub(super) fn poll(&mut self) -> Vec<SvcMessage> {
        self.abandoned.retain(|slot| !slot.is_finished());

        let now = Instant::now();
        let retired = self
            .pending
            .iter()
            .filter_map(|(operation_id, io)| {
                let overdue = io.cancel_deadline.is_some_and(|deadline| deadline <= now);
                (io.slot.is_finished() || overdue).then_some(*operation_id)
            })
            .collect::<Vec<_>>();
        let mut messages = Vec::with_capacity(retired.len());
        for operation_id in retired {
            let io = self
                .pending
                .remove(&operation_id)
                .expect("operation was just listed as pending");
            if io.slot.is_finished() {
                messages.push(io.slot.completion(io.kind, io.request));
            } else {
                warn!(
                    device_id = io.device_id,
                    completion_id = io.request.completion_id,
                    "Port driver did not complete a cancelled request; completing it as cancelled"
                );
                // The worker keeps the request and its host lifetime guard until the driver completes it.
                self.abandoned.push(io.slot);
                messages.push(failure_completion(io.kind, io.request, NtStatus::CANCELLED));
            }
        }

        let pending = &self.pending;
        self.flushes.retain(|flush| {
            if flush
                .writes
                .iter()
                .any(|operation_id| pending.contains_key(operation_id))
            {
                return true;
            }
            messages.push(flush_completion(flush.request.clone(), NtStatus::SUCCESS));
            false
        });

        let pending = &self.pending;
        self.closes.retain(|close| {
            if pending.values().any(|io| io.file_id == close.file_id) {
                return true;
            }
            messages.push(close_completion(close.clone()));
            false
        });
        messages
    }

    /// Discards the files and pending requests of a port the server rejected.
    ///
    /// The server can open a port before the announcement response, and the processor ignores
    /// IRPs for the port once it is rejected, closes included. Nothing is completed and nothing
    /// waits for the driver: cancelled requests are left to their workers.
    pub(super) fn release_device(&mut self, device_id: u32) {
        let abandoned = &mut self.abandoned;
        self.pending.retain(|_, io| {
            if io.device_id != device_id {
                return true;
            }
            cancel_request(&io.handle, io.io_status_block);
            if !io.slot.is_finished() {
                abandoned.push(Arc::clone(&io.slot));
            }
            false
        });
        self.flushes.retain(|flush| flush.request.device_id != device_id);
        self.closes.retain(|close| close.device_id != device_id);
        let open_files = self.files.len();
        self.files.retain(|_, file| file.device_id != device_id);
        if open_files != self.files.len() {
            debug!(device_id, "Released the files of a rejected port");
        }
    }

    /// Discards every open file and pending request for a new RDPDR initialization sequence.
    ///
    /// Nothing waits for the driver: cancelled requests are left to their workers.
    pub(super) fn reset(&mut self) {
        for (_, io) in self.pending.drain() {
            cancel_request(&io.handle, io.io_status_block);
            if !io.slot.is_finished() {
                self.abandoned.push(io.slot);
            }
        }
        self.flushes.clear();
        self.closes.clear();
        self.files.clear();
    }
}

#[cfg(test)]
impl PortSession {
    /// Configures one port opened from an arbitrary NT path instead of its DOS device.
    ///
    /// The port is parallel, so opening it does not apply serial defaults.
    fn with_device_path(device_id: u32, path: String) -> Self {
        let mut session = Self::new(Vec::new(), None);
        session.ports.insert(
            device_id,
            ConfiguredPort {
                kind: RdpdrPortKind::Parallel,
                name: "PIPE".to_owned(),
                path,
            },
        );
        session
    }
}

impl Default for PortSession {
    fn default() -> Self {
        Self::new(Vec::new(), None)
    }
}

impl Drop for PortSession {
    fn drop(&mut self) {
        self.reset();
    }
}

fn cancel_request(handle: &OwnedHandle, io_status_block: usize) {
    let mut cancel_status = IO_STATUS_BLOCK::default();
    // SAFETY: the handle is open. NtCancelIoFileEx only compares the status block address with
    // requests pending on the handle; it does not dereference it.
    let _ = unsafe {
        NtCancelIoFileEx(
            handle.0,
            Some(io_status_block as *const IO_STATUS_BLOCK),
            &mut cancel_status,
        )
    };
}

fn failure_completion(kind: PortIoKind, request: DeviceIoRequest, status: NtStatus) -> SvcMessage {
    let reply = DeviceIoResponse::new(request.clone(), status);
    match kind {
        PortIoKind::Read => SvcMessage::from(RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
            device_io_reply: reply,
            read_data: Vec::new(),
        })),
        PortIoKind::Write => SvcMessage::from(RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
            device_io_reply: reply,
            length: 0,
        })),
        PortIoKind::DeviceControl(code) => SvcMessage::from(RdpdrPdu::DeviceControlResponse(
            DeviceControlResponse::with_raw_output(request, status, control_output(code, Vec::new())),
        )),
    }
}

fn close_completion(request: DeviceIoRequest) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: DeviceIoResponse::new(request, NtStatus::SUCCESS),
    }))
}

fn flush_completion(request: DeviceIoRequest, status: NtStatus) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceFlushBuffersResponse(DeviceFlushBuffersResponse {
        device_io_response: DeviceIoResponse::new(request, status),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_names_are_normalized() {
        assert_eq!(normalize_port_name("com3"), Some("COM3".to_owned()));
        assert_eq!(normalize_port_name(r"\\.\COM10"), Some("COM10".to_owned()));
        assert_eq!(normalize_port_name(r"\DosDevices\LPT1"), Some("LPT1".to_owned()));
        assert_eq!(normalize_port_name("LPT2:"), Some("LPT2".to_owned()));
        assert_eq!(normalize_port_name(r"COM1\..\C"), None);
        assert_eq!(normalize_port_name(""), None);
    }

    #[test]
    fn ports_sort_numerically() {
        let mut names = vec!["COM10".to_owned(), "COM2".to_owned(), "COM1".to_owned()];
        names.sort_by_key(|name| port_sort_key(name));
        assert_eq!(names, ["COM1", "COM2", "COM10"]);
    }

    #[test]
    fn named_selection_skips_missing_ports() {
        let mut next = 100;
        let ports = resolve_ports(
            &WindowsPortRedirection {
                serial: PortSelection::Named(vec!["COM250".to_owned()]),
                parallel: PortSelection::None,
            },
            || {
                next += 1;
                Some(next)
            },
        );
        assert!(ports.is_empty());
    }

    #[test]
    fn unknown_file_ids_complete_with_invalid_handle() {
        use ironrdp_rdpdr::pdu::efs::{DeviceReadRequest, MajorFunction, MinorFunction};

        let mut session = PortSession::new(
            vec![RdpdrPort::new(7, RdpdrPortKind::Serial, "COM250".to_owned()).expect("valid port name")],
            None,
        );
        let messages = session.handle(PortIoRequest::Read(DeviceReadRequest {
            device_io_request: DeviceIoRequest {
                device_id: 7,
                file_id: 3,
                completion_id: 9,
                major_function: MajorFunction::Read,
                minor_function: MinorFunction::from(0),
            },
            length: 16,
            offset: 0,
        }));
        assert_eq!(messages.len(), 1);
        let encoded = messages[0].encode_unframed_pdu().expect("encode read completion");
        assert_eq!(
            u32::from_le_bytes(encoded[12..16].try_into().expect("status")),
            u32::from(NtStatus::INVALID_HANDLE)
        );
    }

    #[test]
    fn opening_a_missing_port_fails_with_the_driver_status() {
        let mut session = PortSession::new(
            vec![RdpdrPort::new(7, RdpdrPortKind::Serial, "COM250".to_owned()).expect("valid port name")],
            None,
        );
        let response = session.handle(create_request(1));
        let response = response[0].encode_unframed_pdu().expect("encode create completion");
        assert_ne!(status_of(&response), NtStatus::SUCCESS);
        assert_eq!(u32::from_le_bytes(response[16..20].try_into().expect("file ID")), 0);
        assert!(session.files.is_empty());
    }

    #[test]
    fn supersede_and_overwrite_creates_are_rejected() {
        use ironrdp_rdpdr::pdu::efs::CreateDisposition;

        let mut session = PortSession::with_device_path(7, r"\??\pipe\ironrdp-port-test-unused".to_owned());
        for disposition in [CreateDisposition::FILE_SUPERSEDE, CreateDisposition::FILE_OVERWRITE] {
            let PortIoRequest::Create(mut create) = create_request(1) else {
                unreachable!("create_request builds a create");
            };
            create.create_disposition = disposition;
            let response = session.handle(PortIoRequest::Create(create));
            assert_eq!(
                status_of(&response[0].encode_unframed_pdu().expect("encode create completion")),
                NtStatus::INVALID_PARAMETER
            );
        }
        assert!(session.files.is_empty());
    }

    fn io_request(
        major_function: ironrdp_rdpdr::pdu::efs::MajorFunction,
        file_id: u32,
        completion_id: u32,
    ) -> DeviceIoRequest {
        DeviceIoRequest {
            device_id: 7,
            file_id,
            completion_id,
            major_function,
            minor_function: ironrdp_rdpdr::pdu::efs::MinorFunction::from(0),
        }
    }

    fn status_of(encoded: &[u8]) -> NtStatus {
        NtStatus::from(u32::from_le_bytes(encoded[12..16].try_into().expect("status")))
    }

    /// Sends a close and collects its completion and everything completed before it.
    ///
    /// The close itself completes through poll once the cancelled requests have finished.
    fn close_and_collect(session: &mut PortSession, file_id: u32, completion_id: u32) -> Vec<Vec<u8>> {
        let mut messages = session
            .handle(close_request(file_id, completion_id))
            .into_iter()
            .map(|message| message.encode_unframed_pdu().expect("encode completion"))
            .collect::<Vec<_>>();
        let deadline = Instant::now() + Duration::from_secs(5);
        while messages
            .last()
            .is_none_or(|last| completion_id_of(last) != completion_id)
        {
            assert!(Instant::now() < deadline, "close did not complete");
            thread::sleep(Duration::from_millis(5));
            messages.extend(
                session
                    .poll()
                    .into_iter()
                    .map(|message| message.encode_unframed_pdu().expect("encode completion")),
            );
        }
        messages
    }

    fn poll_until_completed(session: &mut PortSession) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut messages = session.poll();
            if let Some(message) = messages.pop() {
                assert!(messages.is_empty());
                return message.encode_unframed_pdu().expect("encode deferred completion");
            }
            assert!(Instant::now() < deadline, "deferred port request did not complete");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// A create with the read and write access a server application asks for.
    fn create_request(completion_id: u32) -> PortIoRequest {
        use ironrdp_rdpdr::pdu::efs::{
            CreateDisposition, CreateOptions, DesiredAccess, DeviceCreateRequest, FileAttributes, MajorFunction,
            SharedAccess,
        };

        PortIoRequest::Create(DeviceCreateRequest {
            device_io_request: io_request(MajorFunction::Create, 0, completion_id),
            // FILE_GENERIC_READ | FILE_GENERIC_WRITE
            desired_access: DesiredAccess::from_bits_retain(0x0012_019F),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::empty(),
            path: String::new(),
        })
    }

    fn create_file(session: &mut PortSession, completion_id: u32) -> u32 {
        let create = session.handle(create_request(completion_id));
        let create = create[0].encode_unframed_pdu().expect("encode create completion");
        assert_eq!(status_of(&create), NtStatus::SUCCESS);
        let file_id = u32::from_le_bytes(create[16..20].try_into().expect("file ID"));
        assert_ne!(file_id, 0);
        file_id
    }

    /// A named pipe stands in for a serial port: like a port it is opened for asynchronous I/O,
    /// pends reads until the peer writes, and cancels them on close.
    ///
    /// Returns the pipe's server end, a session whose device 7 is the client end, and the first
    /// server file ID opened on it.
    fn open_pipe_port() -> (OwnedHandle, PortSession, u32) {
        use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_TYPE_BYTE};

        let pipe_name = format!(
            "ironrdp-port-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
                .as_nanos()
        );
        let server_name = wide(&format!(r"\\.\pipe\{pipe_name}"));
        // SAFETY: the name is NUL-terminated and no security attributes are passed.
        let server = unsafe {
            CreateNamedPipeW(
                PCWSTR(server_name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE,
                1,
                4096,
                4096,
                0,
                None,
            )
        };
        assert!(!server.is_invalid(), "create test pipe");
        let server = OwnedHandle(server);

        let mut session = PortSession::with_device_path(7, format!(r"\??\pipe\{pipe_name}"));
        let file_id = create_file(&mut session, 1);
        // SAFETY: the server handle is open; the client is already connected, which this reports.
        let _ = unsafe { ConnectNamedPipe(server.0, None) };
        (server, session, file_id)
    }

    fn write_to_pipe(server: &OwnedHandle, data: &[u8]) {
        // SAFETY: the server handle is open and the buffer outlives the synchronous write.
        unsafe { windows::Win32::Storage::FileSystem::WriteFile(server.0, Some(data), None, None) }
            .expect("write to the test pipe");
    }

    fn read_request(file_id: u32, completion_id: u32) -> PortIoRequest {
        PortIoRequest::Read(ironrdp_rdpdr::pdu::efs::DeviceReadRequest {
            device_io_request: io_request(ironrdp_rdpdr::pdu::efs::MajorFunction::Read, file_id, completion_id),
            length: 16,
            offset: 0,
        })
    }

    fn close_request(file_id: u32, completion_id: u32) -> PortIoRequest {
        PortIoRequest::Close(ironrdp_rdpdr::pdu::efs::DeviceCloseRequest {
            device_io_request: io_request(ironrdp_rdpdr::pdu::efs::MajorFunction::Close, file_id, completion_id),
        })
    }

    fn completion_id_of(encoded: &[u8]) -> u32 {
        u32::from_le_bytes(encoded[8..12].try_into().expect("completion ID"))
    }

    #[test]
    fn pipe_backed_port_pends_reads_forwards_writes_and_cancels_on_close() {
        use ironrdp_rdpdr::pdu::efs::{
            DecodedDeviceControlRequest, DeviceControlRequest, DeviceWriteRequest, MajorFunction,
        };
        use ironrdp_rdpdr::pdu::esp::PortIoCtlCode;
        use windows::Win32::Storage::FileSystem::ReadFile;

        let (server, mut session, file_id) = open_pipe_port();

        let read = |completion_id| read_request(file_id, completion_id);

        // The read pends until the peer writes.
        assert!(session.handle(read(2)).is_empty());
        assert!(session.poll().is_empty());
        write_to_pipe(&server, b"hello");
        let completion = poll_until_completed(&mut session);
        assert_eq!(status_of(&completion), NtStatus::SUCCESS);
        assert_eq!(u32::from_le_bytes(completion[16..20].try_into().expect("length")), 5);
        assert_eq!(&completion[20..], b"hello");

        let mut write = session.handle(PortIoRequest::Write(DeviceWriteRequest {
            device_io_request: io_request(MajorFunction::Write, file_id, 3),
            offset: 0,
            write_data: b"ping".to_vec(),
        }));
        let write = match write.pop() {
            Some(write) => write.encode_unframed_pdu().expect("encode write completion"),
            None => poll_until_completed(&mut session),
        };
        assert_eq!(status_of(&write), NtStatus::SUCCESS);
        assert_eq!(u32::from_le_bytes(write[16..20].try_into().expect("length")), 4);
        let mut received = [0u8; 4];
        // SAFETY: the server handle is open and the buffer outlives the synchronous read.
        unsafe { ReadFile(server.0, Some(&mut received), None, None) }.expect("read from the test pipe");
        assert_eq!(&received, b"ping");

        // Serial IOCTLs reach the driver unchanged; a pipe rejects them without output.
        let control = session.handle(PortIoRequest::DeviceControl(DecodedDeviceControlRequest {
            request: DeviceControlRequest {
                header: io_request(MajorFunction::DeviceControl, file_id, 4),
                output_buffer_length: 4,
                input_buffer_length: 0,
                io_control_code: PortIoCtlCode::IOCTL_SERIAL_GET_BAUD_RATE,
            },
            input_buffer: Vec::new(),
        }));
        let control = control[0].encode_unframed_pdu().expect("encode control completion");
        assert_ne!(status_of(&control), NtStatus::SUCCESS);
        assert_eq!(u32::from_le_bytes(control[16..20].try_into().expect("length")), 0);

        let control = |completion_id, code, output_buffer_length| {
            PortIoRequest::DeviceControl(DecodedDeviceControlRequest {
                request: DeviceControlRequest {
                    header: io_request(MajorFunction::DeviceControl, file_id, completion_id),
                    output_buffer_length,
                    input_buffer_length: 0,
                    io_control_code: code,
                },
                input_buffer: Vec::new(),
            })
        };
        // Like mstsc, a wait-on-mask completion always carries a 4-byte mask, even on failure.
        let wait = session.handle(control(10, PortIoCtlCode::IOCTL_SERIAL_WAIT_ON_MASK, 4));
        let wait = wait[0].encode_unframed_pdu().expect("encode wait completion");
        assert_ne!(status_of(&wait), NtStatus::SUCCESS);
        assert_eq!(u32::from_le_bytes(wait[16..20].try_into().expect("length")), 4);
        // Too-short buffers are rejected before reaching the driver, with no output.
        let status = session.handle(control(11, PortIoCtlCode::IOCTL_SERIAL_GET_COMMSTATUS, 18));
        assert_eq!(
            status_of(&status[0].encode_unframed_pdu().expect("encode status completion")),
            NtStatus::BUFFER_TOO_SMALL
        );
        let wait = session.handle(control(12, PortIoCtlCode::IOCTL_SERIAL_WAIT_ON_MASK, 3));
        let wait = wait[0].encode_unframed_pdu().expect("encode rejected wait");
        assert_eq!(status_of(&wait), NtStatus::BUFFER_TOO_SMALL);
        assert_eq!(u32::from_le_bytes(wait[16..20].try_into().expect("length")), 0);

        // Closing cancels the pending read; its completion precedes the close completion.
        assert!(session.handle(read(5)).is_empty());
        let close = close_and_collect(&mut session, file_id, 6);
        assert_eq!(close.len(), 2);
        assert_eq!(completion_id_of(&close[0]), 5);
        assert_eq!(status_of(&close[0]), NtStatus::CANCELLED);
        assert_eq!(status_of(&close[1]), NtStatus::SUCCESS);
        assert!(session.files.is_empty());
        assert!(session.closes.is_empty());
        assert!(session.poll().is_empty());
    }

    /// A request that completed but was not polled yet keeps its status block allocated, so closing
    /// its file reports its real result instead of cancelling an unrelated request.
    #[test]
    fn closing_a_file_reports_a_completed_request_that_was_not_polled() {
        let (server, mut session, file_id) = open_pipe_port();

        assert!(session.handle(read_request(file_id, 3)).is_empty());
        write_to_pipe(&server, b"one");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.pending.values().all(|io| io.slot.is_finished()) {
            assert!(Instant::now() < deadline, "first read did not complete");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(session.handle(read_request(file_id, 4)).is_empty());

        let close = close_and_collect(&mut session, file_id, 5);
        assert_eq!(close.len(), 3);
        let mut reads = close[..2].to_vec();
        reads.sort_by_key(|read| completion_id_of(read));
        assert_eq!(completion_id_of(&reads[0]), 3);
        assert_eq!(status_of(&reads[0]), NtStatus::SUCCESS);
        assert_eq!(&reads[0][20..], b"one");
        assert_eq!(completion_id_of(&reads[1]), 4);
        assert_eq!(status_of(&reads[1]), NtStatus::CANCELLED);
        assert_eq!(completion_id_of(&close[2]), 5);
    }

    /// A new initialization sequence discards pending requests without waiting for the driver.
    #[test]
    fn reset_abandons_pending_requests_without_waiting() {
        let (_server, mut session, file_id) = open_pipe_port();
        assert!(session.handle(read_request(file_id, 2)).is_empty());

        session.reset();
        assert!(session.pending.is_empty());
        assert!(session.files.is_empty());
        // The cancelled read is left to its worker, which finishes on its own.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.abandoned.iter().all(|slot| slot.is_finished()) {
            assert!(Instant::now() < deadline, "cancelled read did not finish");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(session.poll().is_empty());
        assert!(session.abandoned.is_empty());
    }

    /// A port the server opened before rejecting it loses its files and pending requests, since
    /// no close for it will be processed.
    #[test]
    fn releasing_a_rejected_port_closes_its_files_without_completions() {
        let (_server, mut session, file_id) = open_pipe_port();
        assert!(session.handle(read_request(file_id, 2)).is_empty());

        session.release_device(7);
        assert!(session.pending.is_empty());
        assert!(session.files.is_empty());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.abandoned.iter().all(|slot| slot.is_finished()) {
            assert!(Instant::now() < deadline, "cancelled read did not finish");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(session.poll().is_empty());
        assert!(session.abandoned.is_empty());
    }

    /// Like mstsc, every server create opens its own handle, so a second open of an exclusive
    /// device fails as it would locally.
    #[test]
    fn each_create_opens_its_own_handle() {
        let (_server, mut session, file_id) = open_pipe_port();

        // The test pipe has a single instance, which is already connected.
        let second = session.handle(create_request(2));
        let second = second[0].encode_unframed_pdu().expect("encode create completion");
        assert_ne!(status_of(&second), NtStatus::SUCCESS);
        assert_eq!(session.files.len(), 1);
        assert!(session.files.contains_key(&file_id));
    }

    #[test]
    fn deferred_flushes_count_toward_the_pending_limit() {
        use ironrdp_rdpdr::pdu::efs::{DeviceFlushBuffersRequest, DeviceWriteRequest, MajorFunction};

        let (_server, mut session, file_id) = open_pipe_port();
        // The peer never reads, so a write larger than the pipe quota stays pending.
        assert!(
            session
                .handle(PortIoRequest::Write(DeviceWriteRequest {
                    device_io_request: io_request(MajorFunction::Write, file_id, 2),
                    offset: 0,
                    write_data: vec![0x55; 256 * 1024],
                }))
                .is_empty()
        );

        let flush = |completion_id| {
            PortIoRequest::FlushBuffers(DeviceFlushBuffersRequest {
                device_io_request: io_request(MajorFunction::FlushBuffers, file_id, completion_id),
            })
        };
        let mut completion_id = 3;
        for _ in 1..MAX_PENDING_PORT_OPERATIONS {
            assert!(session.handle(flush(completion_id)).is_empty());
            completion_id += 1;
        }
        let rejected = session.handle(flush(completion_id));
        assert_eq!(
            status_of(&rejected[0].encode_unframed_pdu().expect("encode rejected flush")),
            NtStatus::INSUFFICIENT_RESOURCES
        );
        let rejected = session.handle(read_request(file_id, completion_id + 1));
        assert_eq!(
            status_of(&rejected[0].encode_unframed_pdu().expect("encode rejected read")),
            NtStatus::INSUFFICIENT_RESOURCES
        );

        // Closing completes every deferred flush and the write before the close itself.
        let close = close_and_collect(&mut session, file_id, completion_id + 2);
        assert_eq!(close.len(), MAX_PENDING_PORT_OPERATIONS + 1);
        assert!(
            close[..close.len() - 1]
                .iter()
                .all(|completion| status_of(completion) == NtStatus::CANCELLED)
        );
        assert!(session.flushes.is_empty());
        assert!(session.pending.is_empty());
    }
}
