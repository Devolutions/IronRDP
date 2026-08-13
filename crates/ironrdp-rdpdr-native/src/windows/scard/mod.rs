//! Stub Windows smartcard session for RDPDR device ID 0.
//!
//! Completes every decoded MS-RDPESC call with `SCARD_E_UNSUPPORTED_FEATURE`
//! so Device Control IRPs never hang while the real WinSCard backend lands.

use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{DeviceControlRequest, DeviceControlResponse, NtStatus};
use ironrdp_rdpdr::pdu::esc::{LongReturn, ReturnCode, ScardCall, ScardIoCtlCode};
use ironrdp_svc::SvcMessage;

/// Per-connection smartcard state for RDPDR device ID 0.
///
/// This stub holds no native WinSCard resources. It only provides the backend
/// hooks (`new` / `reset` / `poll` / `handle_call`) expected by
/// [`super::backend::WindowsRdpdrBackend`].
#[derive(Debug, Default)]
pub(super) struct ScardSession;

impl ScardSession {
    pub(super) fn new() -> Self {
        Self
    }

    pub(super) fn reset(&mut self) {}

    pub(super) fn poll(&mut self) -> Vec<SvcMessage> {
        Vec::new()
    }

    pub(super) fn handle_call(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        Ok(vec![complete_long(req, ReturnCode::UnsupportedFeature)])
    }
}

fn complete_long(req: DeviceControlRequest<ScardIoCtlCode>, code: ReturnCode) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse::new(
        req,
        NtStatus::SUCCESS,
        Some(Box::new(LongReturn::new(code))),
    )))
}
