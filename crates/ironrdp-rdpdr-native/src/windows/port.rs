//! Windows serial (COM) and parallel (LPT) port redirection for [MS-RDPESP].
//!
//! Each redirected port is opened once with asynchronous native I/O while the server
//! holds at least one file open on it. Serial and parallel devices are exclusive, so all
//! server file IDs for a port share the local handle. Reads, writes and device-control
//! requests are forwarded unchanged to the local driver, and the driver's NTSTATUS and
//! transfer count are returned as-is, as MS-RDPESP Appendix A describes for Windows.
//! Requests the driver pends, such as reads waiting for data or
//! `IOCTL_SERIAL_WAIT_ON_MASK`, complete later through
//! [`PortSession::poll`]; closing a file cancels the requests still pending on it.
//! A flush completes once the writes pending on its port when it arrived have completed,
//! which is what the serial driver's own flush waits for.
//!
//! [MS-RDPESP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesp/

use core::time::Duration;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    DeviceCloseResponse, DeviceControlResponse, DeviceCreateResponse, DeviceFlushBuffersResponse, DeviceIoRequest,
    DeviceIoResponse, DeviceReadResponse, DeviceWriteResponse, Information, NtStatus,
};
use ironrdp_rdpdr::pdu::esp::PortIoRequest;
use ironrdp_rdpdr::{RdpdrPort, RdpdrPortKind};
use ironrdp_svc::SvcMessage;
use tracing::{debug, warn};
use windows::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows::Wdk::Storage::FileSystem::{
    FILE_NON_DIRECTORY_FILE, FILE_OPEN, NtCancelIoFileEx, NtCreateFile, NtReadFile, NtWriteFile,
};
use windows::Wdk::System::IO::NtDeviceIoControlFile;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE,
    UNICODE_STRING, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_MODE, QueryDosDeviceW,
};
use windows::Win32::System::IO::IO_STATUS_BLOCK;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ, RegCloseKey, RegEnumValueW, RegOpenKeyExW,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::{PCWSTR, PWSTR};

/// Largest read or device-control output buffer allocated for one server request.
///
/// MS-RDPESP allows partial reads, so longer reads are shortened. Serial and parallel
/// control structures are far smaller.
const MAX_PORT_TRANSFER: usize = 64 * 1024;
const MAX_PENDING_PORT_OPERATIONS: usize = 64;
const MAX_OPEN_PORT_FILES: usize = 64;
/// How long a close waits for the driver to complete a cancelled request.
const CANCELLATION_TIMEOUT: Duration = Duration::from_secs(2);
const STATUS_PENDING: NTSTATUS = NTSTATUS(0x0000_0103);

const SERIALCOMM_KEY: &str = r"HARDWARE\DEVICEMAP\SERIALCOMM";
const PARALLEL_PORTS_KEY: &str = r"HARDWARE\DEVICEMAP\PARALLEL PORTS";

/// Which local ports of one kind to redirect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PortSelection {
    /// Redirect no port of this kind.
    #[default]
    None,
    /// Redirect every port of this kind present when the connection starts.
    All,
    /// Redirect the named ports, such as `COM3` or `LPT1`, that are present when the connection starts.
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

    /// Returns whether any port may be redirected.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        !self.serial.is_none() || !self.parallel.is_none()
    }
}

/// A serial or parallel port present on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPort {
    /// Whether the port is serial or parallel.
    pub kind: RdpdrPortKind,
    /// The DOS device name, such as `COM3` or `LPT1`.
    pub name: String,
}

/// Lists the serial and parallel ports on this computer.
///
/// Ports are the `COM<n>` and `LPT<n>` DOS devices, like the names mstsc probes, plus the
/// names port drivers publish in `HKLM\HARDWARE\DEVICEMAP\SERIALCOMM` and
/// `HKLM\HARDWARE\DEVICEMAP\PARALLEL PORTS`. Names without a DOS device link are skipped.
pub fn local_ports() -> Vec<LocalPort> {
    let dos_devices = dos_device_names();
    let mut ports = Vec::new();
    for (kind, prefix, key) in [
        (RdpdrPortKind::Serial, "COM", SERIALCOMM_KEY),
        (RdpdrPortKind::Parallel, "LPT", PARALLEL_PORTS_KEY),
    ] {
        let numbered = dos_devices.iter().filter(|name| {
            name.strip_prefix(prefix)
                .is_some_and(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
        });
        let mapped = registry_string_values(key)
            .into_iter()
            .filter_map(|name| normalize_port_name(&name))
            .filter(|name| dos_device_exists(name));
        let mut names = numbered.cloned().chain(mapped).collect::<Vec<_>>();
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
                    // Virtual port drivers do not always publish their names in the device map.
                    if local.iter().any(|port| port.kind == kind && port.name == name) || dos_device_exists(&name) {
                        selected.push((kind, name));
                    } else {
                        warn!(port = %name, "Ignoring port that is not present on this computer");
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

fn dos_device_exists(name: &str) -> bool {
    let name = wide(name);
    let mut target = [0u16; 512];
    // SAFETY: `name` is a NUL-terminated UTF-16 string and `target` is a writable buffer whose
    // length the binding passes alongside it.
    let length = unsafe { QueryDosDeviceW(PCWSTR(name.as_ptr()), Some(&mut target)) };
    length != 0
}

fn registry_string_values(subkey: &str) -> Vec<String> {
    struct RegistryKey(HKEY);

    impl Drop for RegistryKey {
        fn drop(&mut self) {
            // SAFETY: the key was opened by RegOpenKeyExW and is closed exactly once here.
            let _ = unsafe { RegCloseKey(self.0) };
        }
    }

    let subkey = wide(subkey);
    let mut key = HKEY::default();
    // SAFETY: `subkey` is NUL-terminated and `key` is valid writable storage for the opened key.
    let status = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(subkey.as_ptr()), None, KEY_READ, &mut key) };
    if status != ERROR_SUCCESS {
        // The key only exists while at least one port of that kind is installed.
        debug!(key = %String::from_utf16_lossy(&subkey[..subkey.len() - 1]), ?status, "No port device map");
        return Vec::new();
    }
    let key = RegistryKey(key);

    let mut values = Vec::new();
    for index in 0u32.. {
        let mut value_name = [0u16; 256];
        let mut value_name_length = u32::try_from(value_name.len()).expect("buffer length fits in u32");
        let mut value_type = 0u32;
        let mut data = [0u16; 256];
        let mut data_length = u32::try_from(size_of_val(&data)).expect("buffer size fits in u32");
        // SAFETY: every out pointer refers to writable local storage, and the length arguments
        // describe the capacity of their buffers in characters and bytes respectively.
        let status = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(value_name.as_mut_ptr())),
                &mut value_name_length,
                None,
                Some(&mut value_type),
                Some(data.as_mut_ptr().cast()),
                Some(&mut data_length),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status == ERROR_MORE_DATA {
            continue;
        }
        if status != ERROR_SUCCESS {
            warn!(?status, index, "Failed to enumerate port device map");
            break;
        }
        if value_type != REG_SZ.0 {
            continue;
        }
        let units = usize::try_from(data_length).expect("u32 fits in usize") / size_of::<u16>();
        let value = &data[..units.min(data.len())];
        let value = value.split(|unit| *unit == 0).next().unwrap_or_default();
        values.push(String::from_utf16_lossy(value));
    }
    values
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

fn open_port(path: &str) -> Result<OwnedHandle, NtStatus> {
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
    // - without FILE_SYNCHRONOUS_IO_* the handle is opened for asynchronous I/O, and a zero
    //   share mode requests the exclusive access serial and parallel drivers require.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE,
            &object_attributes,
            &mut io_status,
            None,
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_MODE(0),
            FILE_OPEN,
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
    DeviceControl,
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
    unsafe fn submit(&mut self, handle: HANDLE, io_control_code: u32) -> NTSTATUS {
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
            PortIoKind::DeviceControl => unsafe {
                NtDeviceIoControlFile(
                    handle,
                    Some(self.event.0),
                    None,
                    None,
                    &mut self.io_status,
                    io_control_code,
                    (!self.input.is_empty()).then(|| self.input.as_ptr().cast()),
                    input_length,
                    (!self.output.is_empty()).then(|| self.output.as_mut_ptr().cast()),
                    output_length,
                )
            },
        }
    }

    /// Blocks until the driver completes the request.
    fn wait(&self) {
        // SAFETY: the event handle is owned by `self` and stays open during the wait.
        let result = unsafe { WaitForSingleObject(self.event.0, INFINITE) };
        debug_assert_eq!(result, WAIT_OBJECT_0, "waiting on an owned event cannot fail");
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
            PortIoKind::DeviceControl => {
                self.output.truncate(transferred.min(self.output.len()));
                SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse::with_raw_output(
                    self.request.clone(),
                    io_status,
                    core::mem::take(&mut self.output),
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
    io_status_block: usize,
    worker: thread::JoinHandle<Option<SvcMessage>>,
}

/// A flush waits for the writes on its device that were pending when it arrived.
#[derive(Debug)]
struct PendingFlush {
    device_id: u32,
    file_id: u32,
    request: DeviceIoRequest,
    writes: Vec<u64>,
}

/// Serial and parallel port state for one RDPDR channel lifetime.
#[derive(Debug)]
pub(super) struct PortSession {
    /// NT device path of each configured port, by device ID.
    ports: HashMap<u32, String>,
    handles: HashMap<u32, Arc<OwnedHandle>>,
    files: HashMap<u32, u32>,
    next_file_id: u32,
    next_operation_id: u64,
    pending: HashMap<u64, PendingPortIo>,
    flushes: Vec<PendingFlush>,
    /// Workers whose cancelled request the driver did not complete in time.
    abandoned: Vec<thread::JoinHandle<Option<SvcMessage>>>,
}

impl PortSession {
    pub(super) fn new(ports: Vec<RdpdrPort>) -> Self {
        Self {
            ports: ports
                .into_iter()
                .map(|port| (port.device_id(), port_device_path(port.name())))
                .collect(),
            handles: HashMap::new(),
            files: HashMap::new(),
            next_file_id: 1,
            next_operation_id: 1,
            pending: HashMap::new(),
            flushes: Vec::new(),
            abandoned: Vec::new(),
        }
    }

    pub(super) fn handle(&mut self, req: PortIoRequest) -> Vec<SvcMessage> {
        let device_id = req.device_io_request().device_id;
        let Some(path) = self.ports.get(&device_id) else {
            return vec![SvcMessage::from(req.failure_response(NtStatus::INVALID_DEVICE_REQUEST))];
        };
        let path = path.clone();

        if !matches!(req, PortIoRequest::Create(_)) {
            let file_id = req.device_io_request().file_id;
            if self.files.get(&file_id) != Some(&device_id) {
                return vec![SvcMessage::from(req.failure_response(NtStatus::INVALID_HANDLE))];
            }
        }

        match req {
            PortIoRequest::Create(create) => {
                vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                    self.create(create.device_io_request, &path),
                ))]
            }
            PortIoRequest::Close(close) => self.close(close.device_io_request),
            PortIoRequest::Read(read) => {
                let length = usize::try_from(read.length)
                    .unwrap_or(usize::MAX)
                    .min(MAX_PORT_TRANSFER);
                self.submit(PortIoKind::Read, read.device_io_request, 0, Vec::new(), length)
            }
            PortIoRequest::Write(write) => {
                self.submit(PortIoKind::Write, write.device_io_request, 0, write.write_data, 0)
            }
            PortIoRequest::DeviceControl(control) => {
                let output_length = usize::try_from(control.request.output_buffer_length)
                    .unwrap_or(usize::MAX)
                    .min(MAX_PORT_TRANSFER);
                let code = u32::from(control.request.io_control_code);
                self.submit(
                    PortIoKind::DeviceControl,
                    control.request.header,
                    code,
                    control.input_buffer,
                    output_length,
                )
            }
            PortIoRequest::FlushBuffers(flush) => self.flush(flush.device_io_request),
        }
    }

    fn create(&mut self, request: DeviceIoRequest, path: &str) -> DeviceCreateResponse {
        let device_id = request.device_id;
        let fail = |request, status| DeviceCreateResponse {
            device_io_reply: DeviceIoResponse::new(request, status),
            file_id: 0,
            information: Information::empty(),
        };
        if self.files.len() == MAX_OPEN_PORT_FILES {
            return fail(request, NtStatus::INSUFFICIENT_RESOURCES);
        }

        if let Entry::Vacant(entry) = self.handles.entry(device_id) {
            match open_port(path) {
                Ok(handle) => {
                    debug!(port = %path, device_id, "Opened redirected port");
                    entry.insert(Arc::new(handle));
                }
                Err(status) => {
                    debug!(port = %path, device_id, ?status, "Failed to open redirected port");
                    return fail(request, status);
                }
            }
        }

        let file_id = self.allocate_file_id();
        self.files.insert(file_id, device_id);
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
            if !self.files.contains_key(&file_id) {
                return file_id;
            }
        }
    }

    fn close(&mut self, request: DeviceIoRequest) -> Vec<SvcMessage> {
        // MS-RDPESP 3.2.5.1.3: the client MUST cancel pending I/O on the port before closing it.
        let mut messages =
            self.cancel(|device_id, file_id| device_id == request.device_id && file_id == request.file_id);
        self.files.remove(&request.file_id);
        if !self.files.values().any(|device_id| *device_id == request.device_id) {
            // Workers still finishing cancelled requests keep their own reference.
            self.handles.remove(&request.device_id);
            debug!(device_id = request.device_id, "Closed redirected port");
        }
        messages.push(SvcMessage::from(RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
            device_io_response: DeviceIoResponse::new(request, NtStatus::SUCCESS),
        })));
        messages
    }

    fn submit(
        &mut self,
        kind: PortIoKind,
        request: DeviceIoRequest,
        io_control_code: u32,
        input: Vec<u8>,
        output_length: usize,
    ) -> Vec<SvcMessage> {
        let handle = Arc::clone(
            self.handles
                .get(&request.device_id)
                .expect("open files keep their port handle"),
        );
        if self.pending.len() == MAX_PENDING_PORT_OPERATIONS {
            return vec![failure_completion(kind, request, NtStatus::INSUFFICIENT_RESOURCES)];
        }
        let mut io = match PortIo::new(kind, request.clone(), input, output_length) {
            Ok(io) => io,
            Err(status) => return vec![failure_completion(kind, request, status)],
        };

        // SAFETY: a pending request stays boxed and owned, first by this function and then by
        // the worker that waits for its event, until the driver completes it.
        let status = unsafe { io.submit(handle.0, io_control_code) };
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

        let operation_id = self.next_operation_id;
        self.next_operation_id = self.next_operation_id.wrapping_add(1);
        let io_status_block = core::ptr::addr_of!(io.io_status) as usize;
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        // The request is handed over only once the worker exists, so a failed spawn leaves it here.
        let (sender, receiver) = mpsc::sync_channel::<Box<PortIo>>(1);
        let spawned = thread::Builder::new()
            .name("ironrdp-rdpdr-port".to_owned())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    let mut io = receiver.recv().ok()?;
                    io.wait();
                    let status = io.final_status();
                    Some(io.completion(status))
                })
            });
        let worker = match spawned {
            Ok(worker) => worker,
            Err(error) => {
                warn!(%error, "Failed to start a port worker; cancelling the request");
                cancel_request(&handle, io_status_block);
                io.wait();
                let status = io.final_status();
                return vec![io.completion(status)];
            }
        };
        if let Err(mpsc::SendError(mut io)) = sender.send(io) {
            // The worker exited before receiving the request, so it is still owned here.
            cancel_request(&handle, io_status_block);
            io.wait();
            let status = io.final_status();
            let _ = worker.join();
            return vec![io.completion(status)];
        }

        self.pending.insert(
            operation_id,
            PendingPortIo {
                device_id: request.device_id,
                file_id: request.file_id,
                kind,
                request,
                handle,
                io_status_block,
                worker,
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
                (io.device_id == request.device_id && io.kind == PortIoKind::Write).then_some(*operation_id)
            })
            .collect::<Vec<_>>();
        if writes.is_empty() {
            return vec![flush_completion(request, NtStatus::SUCCESS)];
        }

        self.flushes.push(PendingFlush {
            device_id: request.device_id,
            file_id: request.file_id,
            request,
            writes,
        });
        Vec::new()
    }

    /// Returns the completions of requests the driver finished since the last poll.
    pub(super) fn poll(&mut self) -> Vec<SvcMessage> {
        self.abandoned.retain(|worker| !worker.is_finished());

        let finished = self
            .pending
            .iter()
            .filter_map(|(operation_id, io)| io.worker.is_finished().then_some(*operation_id))
            .collect::<Vec<_>>();
        let mut messages = Vec::with_capacity(finished.len());
        for operation_id in finished {
            let io = self
                .pending
                .remove(&operation_id)
                .expect("operation was just listed as pending");
            match io.worker.join() {
                Ok(Some(message)) => messages.push(message),
                Ok(None) | Err(_) => {
                    tracing::error!(device_id = io.device_id, "Port worker failed");
                    messages.push(failure_completion(io.kind, io.request, NtStatus::UNSUCCESSFUL));
                }
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
        messages
    }

    /// Cancels the matching pending requests and returns their completions.
    fn cancel(&mut self, matches: impl Fn(u32, u32) -> bool) -> Vec<SvcMessage> {
        let operation_ids = self
            .pending
            .iter()
            .filter_map(|(operation_id, io)| matches(io.device_id, io.file_id).then_some(*operation_id))
            .collect::<Vec<_>>();
        let mut messages = Vec::with_capacity(operation_ids.len());
        for operation_id in operation_ids {
            let io = self
                .pending
                .remove(&operation_id)
                .expect("operation was just listed as pending");
            cancel_request(&io.handle, io.io_status_block);
            messages.push(self.finish_cancelled(io));
        }
        self.flushes.retain(|flush| {
            if matches(flush.device_id, flush.file_id) {
                messages.push(flush_completion(flush.request.clone(), NtStatus::CANCELLED));
                false
            } else {
                true
            }
        });
        messages
    }

    /// Waits for a cancelled request; the driver reports its actual outcome, usually STATUS_CANCELLED.
    fn finish_cancelled(&mut self, io: PendingPortIo) -> SvcMessage {
        let deadline = Instant::now() + CANCELLATION_TIMEOUT;
        while !io.worker.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        if !io.worker.is_finished() {
            warn!(
                device_id = io.device_id,
                completion_id = io.request.completion_id,
                "Port driver did not complete a cancelled request; completing it as cancelled"
            );
            self.abandoned.push(io.worker);
            return failure_completion(io.kind, io.request, NtStatus::CANCELLED);
        }
        match io.worker.join() {
            Ok(Some(message)) => message,
            Ok(None) | Err(_) => {
                tracing::error!(device_id = io.device_id, "Port worker failed");
                failure_completion(io.kind, io.request, NtStatus::CANCELLED)
            }
        }
    }

    /// Discards every open file and pending request for a new RDPDR initialization sequence.
    pub(super) fn reset(&mut self) {
        let _ = self.cancel(|_, _| true);
        self.files.clear();
        self.handles.clear();
    }
}

#[cfg(test)]
impl PortSession {
    /// Configures one port opened from an arbitrary NT path instead of its DOS device.
    fn with_device_path(device_id: u32, path: String) -> Self {
        let mut session = Self::new(Vec::new());
        session.ports.insert(device_id, path);
        session
    }
}

impl Default for PortSession {
    fn default() -> Self {
        Self::new(Vec::new())
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
        PortIoKind::DeviceControl => SvcMessage::from(RdpdrPdu::DeviceControlResponse(
            DeviceControlResponse::with_raw_output(request, status, Vec::new()),
        )),
    }
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

        let mut session = PortSession::new(vec![
            RdpdrPort::new(7, RdpdrPortKind::Serial, "COM250".to_owned()).expect("valid port name"),
        ]);
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
        use ironrdp_rdpdr::pdu::efs::{MajorFunction, MinorFunction};

        let mut session = PortSession::new(vec![
            RdpdrPort::new(7, RdpdrPortKind::Serial, "COM250".to_owned()).expect("valid port name"),
        ]);
        let response = session.create(
            DeviceIoRequest {
                device_id: 7,
                file_id: 0,
                completion_id: 1,
                major_function: MajorFunction::Create,
                minor_function: MinorFunction::from(0),
            },
            r"\??\COM250",
        );
        assert_ne!(response.device_io_reply.io_status, NtStatus::SUCCESS);
        assert_eq!(response.file_id, 0);
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

    /// A named pipe stands in for a serial port: like a port it is opened for asynchronous I/O,
    /// pends reads until the peer writes, and cancels them on close.
    #[test]
    fn pipe_backed_port_pends_reads_forwards_writes_and_cancels_on_close() {
        use ironrdp_rdpdr::pdu::efs::{
            CreateDisposition, CreateOptions, DecodedDeviceControlRequest, DesiredAccess, DeviceCloseRequest,
            DeviceControlRequest, DeviceCreateRequest, DeviceReadRequest, DeviceWriteRequest, FileAttributes,
            MajorFunction, SharedAccess,
        };
        use ironrdp_rdpdr::pdu::esp::PortIoCtlCode;
        use windows::Win32::Storage::FileSystem::{PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
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
        let create = session.handle(PortIoRequest::Create(DeviceCreateRequest {
            device_io_request: io_request(MajorFunction::Create, 0, 1),
            desired_access: DesiredAccess::empty(),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::empty(),
            path: String::new(),
        }));
        let create = create[0].encode_unframed_pdu().expect("encode create completion");
        assert_eq!(status_of(&create), NtStatus::SUCCESS);
        let file_id = u32::from_le_bytes(create[16..20].try_into().expect("file ID"));
        assert_ne!(file_id, 0);
        // SAFETY: the server handle is open; the client is already connected, which this reports.
        let _ = unsafe { ConnectNamedPipe(server.0, None) };

        let read = |completion_id| {
            PortIoRequest::Read(DeviceReadRequest {
                device_io_request: io_request(MajorFunction::Read, file_id, completion_id),
                length: 16,
                offset: 0,
            })
        };

        // The read pends until the peer writes.
        assert!(session.handle(read(2)).is_empty());
        assert!(session.poll().is_empty());
        // SAFETY: the server handle is open and the buffer outlives the synchronous write.
        unsafe { WriteFile(server.0, Some(b"hello"), None, None) }.expect("write to the test pipe");
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

        // Closing cancels the pending read before completing the close.
        assert!(session.handle(read(5)).is_empty());
        let close = session.handle(PortIoRequest::Close(DeviceCloseRequest {
            device_io_request: io_request(MajorFunction::Close, file_id, 6),
        }));
        assert_eq!(close.len(), 2);
        let cancelled = close[0].encode_unframed_pdu().expect("encode cancelled read");
        assert_eq!(
            u32::from_le_bytes(cancelled[8..12].try_into().expect("completion ID")),
            5
        );
        assert_eq!(status_of(&cancelled), NtStatus::CANCELLED);
        let closed = close[1].encode_unframed_pdu().expect("encode close completion");
        assert_eq!(u32::from_le_bytes(closed[8..12].try_into().expect("completion ID")), 6);
        assert_eq!(status_of(&closed), NtStatus::SUCCESS);
        assert!(session.handles.is_empty());
        assert!(session.poll().is_empty());
    }
}
