use ironrdp_core::impl_as_any;
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_svc::SvcMessage;

use super::RdpdrBackend;
use crate::pdu::RdpdrPdu;
use crate::pdu::efs::{
    DeviceControlRequest, DeviceIoRequest, DeviceIoResponse, DeviceWriteResponse, NtStatus,
    ServerDeviceAnnounceResponse,
};
use crate::pdu::esc::{ReturnCode, ScardCall, ScardIoCtlCode};

#[derive(Debug)]
pub struct NoopRdpdrBackend;

impl_as_any!(NoopRdpdrBackend);

impl RdpdrBackend for NoopRdpdrBackend {
    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }
    /// Completes every smart card call with `SCARD_E_NO_SERVICE`.
    ///
    /// The server waits for a completion of each IOCTL, so leaving one unanswered would hang
    /// the server-side WinSCard caller.
    fn handle_scard_call(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        let response = call.error_response(req, ReturnCode::NoService);
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceControlResponse(response))])
    }
    fn handle_drive_io_request(&mut self, _req: crate::pdu::efs::ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        Err(pdu_other_err!(
            "filesystem I/O is not supported by the noop RDPDR backend"
        ))
    }

    fn reject_printer_write(&mut self, req: DeviceIoRequest) -> PduResult<Vec<SvcMessage>> {
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(
            DeviceWriteResponse {
                device_io_reply: DeviceIoResponse::new(req, NtStatus::INVALID_PARAMETER),
                length: 0,
            },
        ))])
    }
}
