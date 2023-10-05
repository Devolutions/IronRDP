pub mod noop;

use crate::pdu::{
    efs::{DeviceControlRequest, ServerDeviceAnnounceResponse},
    esc::{ScardCall, ScardIoCtlCode},
};
use core::fmt;
use ironrdp_pdu::PduResult;

/// OS-specific device redirection backend inteface.
pub trait RdpdrBackend: fmt::Debug + Send + Sync {
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()>;
    fn handle_scard_call(&mut self, req: DeviceControlRequest<ScardIoCtlCode>, call: ScardCall) -> PduResult<()>;
}
