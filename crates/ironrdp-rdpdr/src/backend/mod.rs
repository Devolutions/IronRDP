pub mod noop;

use core::fmt;

use ironrdp_core::AsAny;
use ironrdp_pdu::PduResult;
use ironrdp_svc::SvcMessage;

use crate::pdu::efs::{DeviceControlRequest, PrinterIoRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest};
use crate::pdu::esc::{ScardCall, ScardIoCtlCode};

/// OS-specific device redirection backend interface.
pub trait RdpdrBackend: AsAny + fmt::Debug + Send {
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()>;
    fn handle_scard_call(&mut self, req: DeviceControlRequest<ScardIoCtlCode>, call: ScardCall) -> PduResult<()>;
    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>>;
    /// Handle a server-initiated IRP addressed to a printer device.
    ///
    /// `req` carries the fully-decoded printer IRP. Printers only see
    /// [`PrinterIoRequest::Create`] / [`PrinterIoRequest::Write`] /
    /// [`PrinterIoRequest::Close`] on the happy path; any other major
    /// function arrives as [`PrinterIoRequest::Unsupported`] and
    /// should be NAK'd with `STATUS_NOT_IMPLEMENTED`.
    ///
    /// Return the PDUs to send back on the RDPDR channel —
    /// typically a [`crate::pdu::efs::DeviceIoResponse`]-wrapped
    /// `DeviceCreateResponse` / `DeviceWriteResponse` /
    /// `DeviceCloseResponse`. Returning an empty `Vec` is allowed
    /// when the backend has already queued a response out of band
    /// and/or wants to defer.
    fn handle_printer_io_request(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>>;
}
