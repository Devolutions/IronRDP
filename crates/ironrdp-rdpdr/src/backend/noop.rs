use super::RdpdrBackend;
use crate::pdu::efs::{DeviceIoRequest, ServerDeviceAnnounceResponse};
use ironrdp_pdu::PduResult;

#[derive(Debug)]
pub struct NoopRdpdrBackend;

impl RdpdrBackend for NoopRdpdrBackend {
    fn handle_server_device_announce_response(&self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }
    fn handle_device_io_request(&self, _pdu: DeviceIoRequest) -> PduResult<()> {
        Ok(())
    }
}
