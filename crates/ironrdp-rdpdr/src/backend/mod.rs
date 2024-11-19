pub mod noop;

use core::fmt;

use ironrdp_core::AsAny;
use ironrdp_pdu::PduResult;
use ironrdp_svc::SvcMessage;

use crate::pdu::efs::{DeviceControlRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest};
use crate::pdu::esc::{ScardCall, ScardIoCtlCode};

/// OS-specific device redirection backend interface.
pub trait RdpdrBackend: AsAny + fmt::Debug + Send {
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()>;
    fn handle_scard_call(&mut self, req: DeviceControlRequest<ScardIoCtlCode>, call: ScardCall) -> PduResult<()>;
    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>>;
}
