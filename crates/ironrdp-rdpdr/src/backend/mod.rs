pub mod noop;

use core::fmt;
use std::sync::Arc;

use ironrdp_core::AsAny;
use ironrdp_pdu::PduResult;
use ironrdp_svc::SvcMessage;

use crate::Rdpdr;
use crate::pdu::RdpdrPdu;
use crate::pdu::efs::{
    AnyIoCtlCode, DecodedDeviceControlRequest, DeviceCloseResponse, DeviceControlRequest, DeviceIoResponse, NtStatus,
    PrinterIoRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
};
use crate::pdu::esc::{ScardCall, ScardIoCtlCode};
use crate::pdu::esp::PortIoRequest;

/// Device redirection backend interface.
pub trait RdpdrBackend: AsAny + fmt::Debug + Send {
    /// Indicates whether the backend implements the RDPDR security IRPs.
    ///
    /// The channel advertises the corresponding optional capability bits only
    /// when this returns `true`.
    fn supports_drive_security(&self) -> bool {
        false
    }

    /// Releases state associated with the current RDPDR initialization sequence.
    ///
    /// A Server Announce Request starts a new sequence. Stateful
    /// implementations must override this method to discard deferred operations
    /// before the channel restores configured drives and announces them again.
    /// Stateless implementations may use the default.
    fn reset(&mut self) -> PduResult<()> {
        Ok(())
    }

    /// Restores backend state for a configured filesystem device after [`Self::reset`].
    ///
    /// The channel calls this before the device can be announced in the new
    /// sequence.
    fn restore_drive(&mut self, _device_id: u32) -> PduResult<()> {
        Ok(())
    }

    /// Activates a filesystem device before its dynamic announcement.
    ///
    /// The channel validates that the device ID is not live and only sends an
    /// announcement after this method succeeds.
    fn add_drive(&mut self, _device_id: u32) -> PduResult<()> {
        Err(ironrdp_pdu::pdu_other_err!(
            "dynamic drive activation is not supported by this RDPDR backend"
        ))
    }

    /// Releases a filesystem device before its removal announcement.
    ///
    /// The returned completions cancel any deferred IRPs and are sent before
    /// the device removal PDU.
    fn remove_drive(&mut self, _device_id: u32) -> PduResult<Vec<SvcMessage>> {
        Err(ironrdp_pdu::pdu_other_err!(
            "dynamic drive removal is not supported by this RDPDR backend"
        ))
    }

    /// Handles the server result for a device announcement.
    ///
    /// A rejected filesystem device is never eligible for server I/O.
    ///
    /// Port IRPs are served before the response arrives, so a backend that opened files for a
    /// port the server then rejects should release them here: the SVC processor ignores the
    /// rejected device's later closes.
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()>;

    /// Handles a decoded smart-card Device Control IRP.
    ///
    /// Return the RDPDR completions to send immediately. Long-running calls such
    /// as status-change waits may return an empty vector and complete later via
    /// [`Self::poll_deferred_messages`].
    fn handle_scard_call(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>>;

    /// Handles a decoded filesystem IRP.
    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>>;

    /// Handles a filesystem Device Control request and its exact opaque input.
    ///
    /// The default preserves the existing `handle_drive_io_request` contract
    /// for backends that do not consume control input.
    fn handle_drive_device_control(
        &mut self,
        req: DecodedDeviceControlRequest<AnyIoCtlCode>,
    ) -> PduResult<Vec<SvcMessage>> {
        self.handle_drive_io_request(ServerDriveIoRequest::DeviceControlRequest(req.request))
    }

    /// Drains completions for filesystem IRPs that were deferred by the backend.
    ///
    /// Implementations must return every completion only once.
    fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    fn handle_user_logged_on(&mut self, _rdpdr: &mut Rdpdr) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    /// Handle a server-initiated IRP addressed to a printer device.
    ///
    /// `req` carries the fully-decoded printer IRP. Printers only see
    /// [`PrinterIoRequest::Create`] / [`PrinterIoRequest::Write`] /
    /// [`PrinterIoRequest::Close`] on the backend path. Unsupported printer
    /// major functions are completed by the SVC processor before the backend
    /// is called.
    ///
    /// Return the PDUs to send back on the RDPDR channel —
    /// typically a [`crate::pdu::efs::DeviceIoResponse`]-wrapped
    /// `DeviceCreateResponse` / `DeviceWriteResponse` /
    /// `DeviceCloseResponse`. Returning an empty `Vec` is allowed
    /// when the backend has already queued a response out of band
    /// and/or wants to defer.
    fn handle_printer_io_request(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        let device_io_request = req.into_device_io_request();
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
            DeviceCloseResponse {
                device_io_response: DeviceIoResponse::new(device_io_request, NtStatus::NOT_SUPPORTED),
            },
        ))])
    }

    /// Handles a server-initiated IRP addressed to a serial or parallel port device.
    ///
    /// Ports see create, close, read, write, flush and device-control requests; other major
    /// functions are completed by the SVC processor before the backend is called. Long-running
    /// requests, such as reads waiting for data or `IOCTL_SERIAL_WAIT_ON_MASK`, may return an
    /// empty vector and complete later via [`Self::poll_deferred_messages`]. A close MUST first
    /// complete every request still pending on that file.
    ///
    /// The default completes every request with `STATUS_NOT_SUPPORTED`.
    fn handle_port_io_request(&mut self, req: PortIoRequest) -> PduResult<Vec<SvcMessage>> {
        Ok(vec![SvcMessage::from(req.failure_response(NtStatus::NOT_SUPPORTED))])
    }

    /// Rejects an oversized printer write before its server-controlled payload is allocated.
    ///
    /// Stateful printer implementations should also poison or abort the target file handle so a
    /// later close cannot submit a partial job.
    fn reject_printer_write(&mut self, req: crate::pdu::efs::DeviceIoRequest) -> PduResult<Vec<SvcMessage>> {
        let _ = req;
        Err(ironrdp_pdu::pdu_other_err!(
            "printer backend does not support safe oversized-write rejection"
        ))
    }
}

/// Immutable filesystem-device metadata announced for one RDPDR channel lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RdpdrDrive {
    device_id: u32,
    name: String,
}

impl RdpdrDrive {
    /// Creates filesystem-device metadata for an RDPDR announcement.
    pub fn new(device_id: u32, name: String) -> Self {
        Self { device_id, name }
    }

    /// Returns the device identifier used in RDPDR requests.
    pub fn device_id(&self) -> u32 {
        self.device_id
    }

    /// Returns the user-visible name sent in the filesystem-device announcement.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Splits the metadata into the RDPDR announcement fields.
    pub fn into_parts(self) -> (u32, String) {
        (self.device_id, self.name)
    }
}

/// Immutable printer metadata announced for one RDPDR channel lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RdpdrPrinter {
    device_id: u32,
    name: String,
    driver_name: String,
    network: bool,
    default: bool,
    explicit_driver: bool,
    server_name: Option<String>,
    cached_config_data: Vec<u8>,
}

impl RdpdrPrinter {
    /// Creates printer metadata for an RDPDR announcement.
    ///
    /// The printer is not marked as the default printer. When a product announces a single
    /// printer without the flag, `ironrdp-client` marks it as the default.
    pub fn new(device_id: u32, name: String, driver_name: String) -> Self {
        Self {
            device_id,
            name,
            driver_name,
            network: true,
            default: false,
            explicit_driver: false,
            server_name: None,
            cached_config_data: Vec::new(),
        }
    }

    /// Records whether the local Windows queue is a network printer.
    #[must_use]
    pub fn with_network(mut self, network: bool) -> Self {
        self.network = network;
        self
    }

    /// Marks the printer as the session default printer.
    ///
    /// [MS-RDPEPC 2.2.2.1] allows only one announced printer with this flag.
    #[must_use]
    pub fn with_default(mut self, default: bool) -> Self {
        self.default = default;
        self
    }

    /// Marks the driver name as chosen by the user.
    ///
    /// A cached driver from an [`RdpdrPrinterCacheStore`] replaces a driver that is not explicit.
    #[must_use]
    pub fn with_explicit_driver(mut self, explicit: bool) -> Self {
        self.explicit_driver = explicit;
        self
    }

    /// Sets the opaque `CachedPrinterConfigData` sent with the announcement.
    #[must_use]
    pub fn with_cached_config_data(mut self, data: Vec<u8>) -> Self {
        self.cached_config_data = data;
        self
    }

    /// Returns the device identifier used in RDPDR requests.
    pub fn device_id(&self) -> u32 {
        self.device_id
    }

    /// Returns the local printer queue name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the printer name announced to the server.
    ///
    /// This is the local queue name unless a cache entry records a name the server gave the
    /// printer; see [`RdpdrPrinterCacheEntry::printer_name`].
    pub fn announced_name(&self) -> &str {
        self.server_name.as_deref().unwrap_or(&self.name)
    }

    /// Returns the Windows printer driver name requested on the server.
    pub fn driver_name(&self) -> &str {
        &self.driver_name
    }

    /// Returns whether the queue should carry the MS-RDPEPC network-printer flag.
    pub fn network(&self) -> bool {
        self.network
    }

    /// Returns whether the printer is announced as the session default printer.
    pub fn is_default(&self) -> bool {
        self.default
    }

    /// Returns whether the driver name was chosen by the user.
    pub fn explicit_driver(&self) -> bool {
        self.explicit_driver
    }

    /// Returns the opaque `CachedPrinterConfigData` sent with the announcement.
    pub fn cached_config_data(&self) -> &[u8] {
        &self.cached_config_data
    }

    /// Applies a cache entry previously saved for this printer.
    ///
    /// The cached printer name replaces the announced name, and the cached driver replaces the
    /// driver name unless it is explicit. The cached configuration
    /// is driver-specific ([MS-RDPEPC 3.1.1.1]): when it belongs to another driver than the one
    /// announced, the printer announces no configuration, and configuration set with
    /// [`Self::with_cached_config_data`] is dropped as well when the cached driver replaced the
    /// driver it was set for.
    ///
    /// [MS-RDPEPC 3.1.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepc/
    #[must_use]
    pub fn with_cache_entry(mut self, entry: RdpdrPrinterCacheEntry) -> Self {
        if entry.printer_name.is_some() {
            self.server_name = entry.printer_name;
        }
        if let Some(driver_name) = entry.driver_name
            && !self.explicit_driver
        {
            self.driver_name = driver_name;
        }
        if entry
            .config_driver_name
            .is_none_or(|config_driver_name| config_driver_name == self.driver_name)
        {
            self.cached_config_data = entry.config_data;
        } else {
            self.cached_config_data.clear();
        }
        self
    }

    /// Splits the metadata into the RDPDR announcement fields.
    ///
    /// Default, server-name, and cached-configuration metadata is not part of the tuple; use
    /// [`Rdpdr::with_redirected_printer`] to announce it.
    pub fn into_parts(self) -> (u32, String, String, bool) {
        (self.device_id, self.name, self.driver_name, self.network)
    }
}

/// Kind of a redirected port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RdpdrPortKind {
    /// A serial (COM) port, announced as `RDPDR_DTYP_SERIAL`.
    Serial,
    /// A parallel (LPT) port, announced as `RDPDR_DTYP_PARALLEL`.
    Parallel,
}

impl RdpdrPortKind {
    /// Returns the announced RDPDR device type.
    pub fn device_type(self) -> crate::pdu::efs::DeviceType {
        match self {
            Self::Serial => crate::pdu::efs::DeviceType::Serial,
            Self::Parallel => crate::pdu::efs::DeviceType::Parallel,
        }
    }
}

/// Immutable serial or parallel port metadata announced for one RDPDR channel lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RdpdrPort {
    device_id: u32,
    kind: RdpdrPortKind,
    name: String,
}

impl RdpdrPort {
    /// Creates port metadata for an RDPDR announcement.
    ///
    /// `name` is the client-side DOS name, such as `COM1` or `LPT1`, and is sent as the
    /// PreferredDosName. It must be 1 to 7 ASCII characters and must not contain any of
    /// `< > " / \ | :` or control characters, which the server rejects.
    pub fn new(device_id: u32, kind: RdpdrPortKind, name: String) -> Result<Self, InvalidPortName> {
        // INVARIANT: 0 < name.len() <= 7, so the null-terminated name fits PreferredDosName.
        let valid = !name.is_empty()
            && name.len() <= 7
            && name.bytes().all(|byte| {
                byte.is_ascii_graphic() && !matches!(byte, b'<' | b'>' | b'"' | b'/' | b'\\' | b'|' | b':')
            });
        if !valid {
            return Err(InvalidPortName);
        }

        Ok(Self { device_id, kind, name })
    }

    /// Returns the device identifier used in RDPDR requests.
    pub fn device_id(&self) -> u32 {
        self.device_id
    }

    /// Returns whether the port is serial or parallel.
    pub fn kind(&self) -> RdpdrPortKind {
        self.kind
    }

    /// Returns the DOS name announced to the server.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Splits the metadata into the RDPDR announcement fields.
    pub fn into_parts(self) -> (u32, RdpdrPortKind, String) {
        (self.device_id, self.kind, self.name)
    }
}

/// A port name that cannot be sent as an RDPDR PreferredDosName.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidPortName;

impl fmt::Display for InvalidPortName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("port name must be 1 to 7 printable ASCII characters without < > \" / \\ | :")
    }
}

impl core::error::Error for InvalidPortName {}

/// Printer configuration a server sent with `PAKID_PRN_CACHE_DATA`.
///
/// [MS-RDPEPC 3.1.1.1] describes it as a record keyed by printer name. The client replays it
/// in the next announcement of the printer so server-side printing preferences persist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RdpdrPrinterCacheEntry {
    /// Name the server gave the printer, from a rename event or a queue a user installed
    /// manually on the redirected port; announced instead of the local queue name.
    pub printer_name: Option<String>,
    /// Server driver of a queue that a user installed manually on the redirected port.
    pub driver_name: Option<String>,
    /// Driver that `config_data` was produced for; `None` applies it to any driver.
    pub config_driver_name: Option<String>,
    /// Opaque `CachedPrinterConfigData` blob.
    pub config_data: Vec<u8>,
}

/// Result returned by an [`RdpdrPrinterCacheStore`].
pub type RdpdrPrinterCacheResult<T> = Result<T, Box<dyn core::error::Error + Send + Sync>>;

/// Persistent store for printer cache data, supplied by the embedder.
///
/// Entries are keyed by the local printer queue name, which stays the same when the server
/// renames the printer; the entry records the new name instead. Embedders decide the scope,
/// for example one store per server host as [MS-RDPEPC 3.2.5.1.3] suggests. The channel logs
/// store errors and continues, so a failing store only loses cached settings.
///
/// [MS-RDPEPC 3.2.5.1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepc/
pub trait RdpdrPrinterCacheStore: Send + Sync {
    /// Loads the entry saved for `printer_name`.
    fn load(&self, printer_name: &str) -> RdpdrPrinterCacheResult<Option<RdpdrPrinterCacheEntry>>;

    /// Saves or replaces the entry for `printer_name`.
    fn save(&self, printer_name: &str, entry: &RdpdrPrinterCacheEntry) -> RdpdrPrinterCacheResult<()>;

    /// Deletes the entry for `printer_name`, if any.
    fn delete(&self, printer_name: &str) -> RdpdrPrinterCacheResult<()>;
}

/// [`RdpdrPrinterCacheStore`] that keeps entries in memory.
///
/// Sharing one instance across reconnects keeps settings for the lifetime of the process.
#[derive(Debug, Default)]
pub struct InMemoryRdpdrPrinterCacheStore {
    entries: std::sync::Mutex<std::collections::BTreeMap<String, RdpdrPrinterCacheEntry>>,
}

impl InMemoryRdpdrPrinterCacheStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<String, RdpdrPrinterCacheEntry>> {
        self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl RdpdrPrinterCacheStore for InMemoryRdpdrPrinterCacheStore {
    fn load(&self, printer_name: &str) -> RdpdrPrinterCacheResult<Option<RdpdrPrinterCacheEntry>> {
        Ok(self.entries().get(printer_name).cloned())
    }

    fn save(&self, printer_name: &str, entry: &RdpdrPrinterCacheEntry) -> RdpdrPrinterCacheResult<()> {
        self.entries().insert(printer_name.to_owned(), entry.clone());
        Ok(())
    }

    fn delete(&self, printer_name: &str) -> RdpdrPrinterCacheResult<()> {
        self.entries().remove(printer_name);
        Ok(())
    }
}

/// Per-connection product created by an [`RdpdrBackendFactory`].
///
/// The initial list is announced during channel initialization.
/// A backend that enables drive hotplug may also contain dormant filesystem devices.
pub struct RdpdrBackendProduct {
    backend: Box<dyn RdpdrBackend>,
    initial_drives: Vec<RdpdrDrive>,
    drive_hotplug: bool,
    ports: Vec<RdpdrPort>,
    printers: Vec<RdpdrPrinter>,
    printer_cache_store: Option<Arc<dyn RdpdrPrinterCacheStore>>,
}

impl RdpdrBackendProduct {
    /// Creates a product for one RDPDR channel lifetime.
    pub fn new(backend: Box<dyn RdpdrBackend>, initial_drives: Vec<RdpdrDrive>) -> Self {
        Self {
            backend,
            initial_drives,
            drive_hotplug: false,
            ports: Vec::new(),
            printers: Vec::new(),
            printer_cache_store: None,
        }
    }

    /// Enables drive capability negotiation even when no drive is announced initially.
    #[must_use]
    pub fn with_drive_hotplug(mut self, enabled: bool) -> Self {
        self.drive_hotplug = enabled;
        self
    }

    /// Returns the filesystem devices announced when the server accepts RDPDR.
    pub fn initial_drives(&self) -> &[RdpdrDrive] {
        &self.initial_drives
    }

    /// Returns whether the channel must remain available for later drive announcements.
    pub fn drive_hotplug(&self) -> bool {
        self.drive_hotplug
    }

    /// Adds a printer announced after the remote user logs on.
    #[must_use]
    pub fn with_printer(mut self, printer: RdpdrPrinter) -> Self {
        self.printers.push(printer);
        self
    }

    /// Adds printers announced after the remote user logs on.
    #[must_use]
    pub fn with_printers(mut self, printers: impl IntoIterator<Item = RdpdrPrinter>) -> Self {
        self.printers.extend(printers);
        self
    }

    /// Returns the first configured printer, if any.
    pub fn printer(&self) -> Option<&RdpdrPrinter> {
        self.printers.first()
    }

    /// Returns all configured printers in announcement order.
    pub fn printers(&self) -> &[RdpdrPrinter] {
        &self.printers
    }

    /// Persists server printer cache data for this product's printers.
    #[must_use]
    pub fn with_printer_cache_store(mut self, store: Arc<dyn RdpdrPrinterCacheStore>) -> Self {
        self.printer_cache_store = Some(store);
        self
    }

    /// Returns the printer cache store, if any.
    pub fn printer_cache_store(&self) -> Option<&Arc<dyn RdpdrPrinterCacheStore>> {
        self.printer_cache_store.as_ref()
    }

    /// Configures serial and parallel ports announced after the remote user logs on.
    #[must_use]
    pub fn with_ports(mut self, ports: Vec<RdpdrPort>) -> Self {
        self.ports = ports;
        self
    }

    /// Returns the configured serial and parallel ports.
    pub fn ports(&self) -> &[RdpdrPort] {
        &self.ports
    }

    /// Consumes the product into its backend and matching announcement metadata.
    pub fn into_parts(self) -> (Box<dyn RdpdrBackend>, Vec<RdpdrDrive>) {
        (self.backend, self.initial_drives)
    }
}

/// Result returned when creating an RDPDR backend for one connection attempt.
pub type RdpdrBackendFactoryResult<T> = Result<T, Box<dyn core::error::Error + Send + Sync>>;

/// Builds a fresh RDPDR backend and device set for each connection attempt.
///
/// Backends own per-session device and file-handle state, so factories must not
/// reuse a product across reconnects.
pub trait RdpdrBackendFactory {
    /// Creates the backend and matching filesystem-device announcements for one RDPDR channel lifetime.
    fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct>;
}
