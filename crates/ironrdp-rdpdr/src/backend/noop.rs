use ironrdp_core::impl_as_any;
use ironrdp_pdu::PduResult;
use ironrdp_svc::SvcMessage;

use super::RdpdrBackend;
use crate::pdu::efs::{DeviceControlRequest, PrinterIoRequest, ServerDeviceAnnounceResponse};
use crate::pdu::esc::{ScardCall, ScardIoCtlCode};

#[derive(Debug)]
pub struct NoopRdpdrBackend;

impl_as_any!(NoopRdpdrBackend);

impl RdpdrBackend for NoopRdpdrBackend {
    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }
    fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
        Ok(())
    }
    fn handle_drive_io_request(&mut self, _req: crate::pdu::efs::ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
    fn handle_printer_io_request(&mut self, _req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}
