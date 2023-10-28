pub mod noop;

use crate::pdu::{
    efs::{DeviceControlRequest, FilesystemRequest, ServerDeviceAnnounceResponse},
    esc::{ScardCall, ScardIoCtlCode},
};
use core::fmt;
use ironrdp_pdu::PduResult;
use ironrdp_svc::AsAny;

/// OS-specific device redirection backend inteface.
pub trait RdpdrBackend: AsAny + fmt::Debug + Send {
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()>;
    fn handle_scard_call(&mut self, req: DeviceControlRequest<ScardIoCtlCode>, call: ScardCall) -> PduResult<()>;
    fn handle_fs_request(&mut self, req: FilesystemRequest) -> PduResult<()>;
}
