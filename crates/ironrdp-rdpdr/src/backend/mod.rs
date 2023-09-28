pub mod noop;

use crate::pdu::efs::{DeviceIoRequest, ServerDeviceAnnounceResponse};
use core::fmt;
use ironrdp_pdu::PduResult;

/// OS-specific device redirection backend inteface.
pub trait RdpdrBackend: fmt::Debug + Send + Sync {
    fn handle_server_device_announce_response(&self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()>;
    fn handle_device_io_request(&self, pdu: DeviceIoRequest) -> PduResult<()>;
}
