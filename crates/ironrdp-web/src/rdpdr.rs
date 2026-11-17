//! The RDPDR backend of the browser client: printers ([`crate::printer`]) and
//! drives ([`crate::drive`]) behind one [`RdpdrBackend`], since a channel has
//! only one.

use ironrdp::rdpdr::backend::RdpdrBackend;
use ironrdp::rdpdr::pdu::efs::{
    DeviceControlRequest, DeviceIoRequest, PrinterIoRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
};
use ironrdp::rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_core::impl_as_any;
use ironrdp_pdu::PduResult;
use ironrdp_svc::SvcMessage;
use tracing::warn;

use crate::drive::WasmDriveBackend;
use crate::printer::WasmPrinterBackend;

#[derive(Debug)]
pub(crate) struct WasmRdpdrBackend {
    printer: Option<WasmPrinterBackend>,
    drive: Option<WasmDriveBackend>,
}

impl_as_any!(WasmRdpdrBackend);

impl WasmRdpdrBackend {
    pub(crate) fn new(printer: Option<WasmPrinterBackend>, drive: Option<WasmDriveBackend>) -> Self {
        Self { printer, drive }
    }

    #[cfg(test)]
    pub(crate) fn printer(&self) -> Option<&WasmPrinterBackend> {
        self.printer.as_ref()
    }

    pub(crate) fn drive_mut(&mut self) -> Option<&mut WasmDriveBackend> {
        self.drive.as_mut()
    }
}

impl RdpdrBackend for WasmRdpdrBackend {
    fn reset(&mut self) -> PduResult<()> {
        if let Some(printer) = &mut self.printer {
            printer.reset()?;
        }
        if let Some(drive) = &mut self.drive {
            drive.reset();
        }
        Ok(())
    }

    fn restore_drive(&mut self, _device_id: u32) -> PduResult<()> {
        // Drives are stateless between channel lifetimes: `reset` closed every handle.
        Ok(())
    }

    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        if let Some(drive) = self.drive.as_mut().filter(|drive| drive.has_drive(pdu.device_id)) {
            drive.announced(pdu.device_id, pdu.result_code);
            return Ok(());
        }
        match &mut self.printer {
            Some(printer) => printer.handle_server_device_announce_response(pdu),
            None => Ok(()),
        }
    }

    fn handle_scard_call(
        &mut self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        warn!("Smartcard IOCTL reached the browser RDPDR backend; ignoring");
        Ok(Vec::new())
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        match &mut self.drive {
            Some(drive) => drive.handle(req),
            None => {
                warn!("Drive IRP without redirected drives; ignoring");
                Ok(Vec::new())
            }
        }
    }

    fn handle_printer_io_request(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        match &mut self.printer {
            Some(printer) => printer.handle_printer_io_request(req),
            None => {
                warn!("Printer IRP without redirected printers; ignoring");
                Ok(Vec::new())
            }
        }
    }

    fn reject_printer_write(&mut self, req: DeviceIoRequest) -> PduResult<Vec<SvcMessage>> {
        match &mut self.printer {
            Some(printer) => printer.reject_printer_write(req),
            None => Err(ironrdp_pdu::pdu_other_err!("printer write without redirected printers")),
        }
    }
}
