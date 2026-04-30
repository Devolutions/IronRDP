pub mod noop;

use core::fmt;

use ironrdp_core::AsAny;
use ironrdp_pdu::PduResult;
use ironrdp_svc::SvcMessage;

use crate::Rdpdr;
use crate::pdu::RdpdrPdu;
use crate::pdu::efs::{
    DeviceCloseResponse, DeviceControlRequest, DeviceIoResponse, NtStatus, PrinterIoRequest,
    ServerDeviceAnnounceResponse, ServerDriveIoRequest,
};
use crate::pdu::esc::{ScardCall, ScardIoCtlCode};

/// OS-specific device redirection backend interface.
pub trait RdpdrBackend: AsAny + fmt::Debug + Send {
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()>;
    fn handle_scard_call(&mut self, req: DeviceControlRequest<ScardIoCtlCode>, call: ScardCall) -> PduResult<()>;
    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>>;

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
}
