pub mod noop;

use core::fmt;

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
}

impl RdpdrPrinter {
    /// Creates printer metadata for an RDPDR announcement.
    pub fn new(device_id: u32, name: String, driver_name: String) -> Self {
        Self {
            device_id,
            name,
            driver_name,
            network: true,
        }
    }

    /// Records whether the local Windows queue is a network printer.
    #[must_use]
    pub fn with_network(mut self, network: bool) -> Self {
        self.network = network;
        self
    }

    /// Returns the device identifier used in RDPDR requests.
    pub fn device_id(&self) -> u32 {
        self.device_id
    }

    /// Returns the local printer queue name announced to the server.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the Windows printer driver name requested on the server.
    pub fn driver_name(&self) -> &str {
        &self.driver_name
    }

    /// Returns whether the queue should carry the MS-RDPEPC network-printer flag.
    pub fn network(&self) -> bool {
        self.network
    }

    /// Splits the metadata into the RDPDR announcement fields.
    pub fn into_parts(self) -> (u32, String, String, bool) {
        (self.device_id, self.name, self.driver_name, self.network)
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
    printer: Option<RdpdrPrinter>,
}

impl RdpdrBackendProduct {
    /// Creates a product for one RDPDR channel lifetime.
    pub fn new(backend: Box<dyn RdpdrBackend>, initial_drives: Vec<RdpdrDrive>) -> Self {
        Self {
            backend,
            initial_drives,
            drive_hotplug: false,
            printer: None,
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

    /// Configures one printer announced after the remote user logs on.
    #[must_use]
    pub fn with_printer(mut self, printer: RdpdrPrinter) -> Self {
        self.printer = Some(printer);
        self
    }

    /// Returns the configured printer, if any.
    pub fn printer(&self) -> Option<&RdpdrPrinter> {
        self.printer.as_ref()
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
