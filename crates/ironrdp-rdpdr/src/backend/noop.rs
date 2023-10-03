use super::RdpdrBackend;
use crate::pdu::{
    efs::{DeviceControlRequest, ServerDeviceAnnounceResponse},
    esc::{ScardAccessStartedEventCall, ScardIoCtlCode},
};
use ironrdp_pdu::PduResult;

#[derive(Debug)]
pub struct NoopRdpdrBackend;

impl RdpdrBackend for NoopRdpdrBackend {
    fn handle_server_device_announce_response(&self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }
    fn handle_scard_access_started_event_call(
        &self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardAccessStartedEventCall,
    ) -> PduResult<()> {
        Ok(())
    }
}
