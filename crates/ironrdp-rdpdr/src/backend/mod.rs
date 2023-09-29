pub mod noop;

use crate::pdu::{
    efs::{DeviceControlRequest, ServerDeviceAnnounceResponse},
    esc::{ScardAccessStartedEventCall, ScardIoctlCode},
};
use core::fmt;
use ironrdp_pdu::PduResult;

/// OS-specific device redirection backend inteface.
pub trait RdpdrBackend: fmt::Debug + Send + Sync {
    fn handle_server_device_announce_response(&self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()>;
    fn handle_scard_access_started_event_call(
        &self,
        req: DeviceControlRequest<ScardIoctlCode>,
        call: ScardAccessStartedEventCall,
    ) -> PduResult<()>;
}
