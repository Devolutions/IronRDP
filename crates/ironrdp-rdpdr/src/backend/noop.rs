use super::RdpdrBackend;
use crate::pdu::{
    efs::{DeviceControlRequest, ServerDeviceAnnounceResponse},
    esc::{ScardCall, ScardIoCtlCode},
};
use ironrdp_pdu::PduResult;

#[derive(Debug)]
pub struct NoopRdpdrBackend;

impl RdpdrBackend for NoopRdpdrBackend {
    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }
    fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
        Ok(())
    }
}
