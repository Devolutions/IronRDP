use ironrdp_pdu::PduResult;
use ironrdp_svc::impl_as_any;

use super::RdpdrBackend;
use crate::pdu::efs::{DeviceControlRequest, ServerDeviceAnnounceResponse};
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
    fn handle_fs_request(&mut self, _req: crate::pdu::efs::FilesystemRequest) -> PduResult<()> {
        Ok(())
    }
}
