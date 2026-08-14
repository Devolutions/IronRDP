//! Stub Windows smartcard session for RDPDR device ID 0.
//!
//! Completes every decoded MS-RDPESC call with the IOCTL-appropriate return
//! PDU and `SCARD_E_UNSUPPORTED_FEATURE`, so peers never hang or mis-decode
//! while the real WinSCard backend lands.

use ironrdp_pdu::PduResult;
use ironrdp_pdu::utils::CharacterSet;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{DeviceControlRequest, DeviceControlResponse, NtStatus};
use ironrdp_rdpdr::pdu::esc::{
    CardProtocol, CardState, ConnectReturn, ControlReturn, EstablishContextReturn, GetAttribReturn,
    GetDeviceTypeIdReturn, GetReaderIconReturn, GetStatusChangeReturn, GetTransmitCountReturn, ListReadersReturn,
    LongReturn, ReadCacheReturn, ReconnectReturn, ReturnCode, ScardCall, ScardContext, ScardHandle, ScardIoCtlCode,
    StateReturn, StatusReturn, TransmitReturn, rpce,
};
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
        call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        let code = ReturnCode::UnsupportedFeature;
        let empty_context = ScardContext::new(0);
        let empty_handle = ScardHandle::new(empty_context, 0);
        let undefined = CardProtocol::SCARD_PROTOCOL_UNDEFINED;

        let message = match call {
            ScardCall::AccessStartedEventCall(_)
            | ScardCall::HCardAndDispositionCall(_)
            | ScardCall::SetAttribCall(_)
            | ScardCall::ContextCall(_)
            | ScardCall::WriteCacheCall(_)
            | ScardCall::ContextAndStringCall(_)
            | ScardCall::ContextAndTwoStringCall(_)
            | ScardCall::Unsupported => complete_rpce(req, Box::new(LongReturn::new(code))),
            ScardCall::EstablishContextCall(_) => {
                complete_rpce(req, Box::new(EstablishContextReturn::new(code, empty_context)))
            }
            ScardCall::ListReaderGroupsCall(_) | ScardCall::ListReadersCall(_) => {
                complete_rpce(req, Box::new(ListReadersReturn::new(code, Vec::new())))
            }
            ScardCall::GetStatusChangeCall(_) | ScardCall::LocateCardsCall(_) | ScardCall::LocateCardsByAtrCall(_) => {
                complete_rpce(req, Box::new(GetStatusChangeReturn::new(code, Vec::new())))
            }
            ScardCall::ConnectCall(_) => {
                complete_rpce(req, Box::new(ConnectReturn::new(code, empty_handle, undefined)))
            }
            ScardCall::ReconnectCall(_) => complete_rpce(req, Box::new(ReconnectReturn::new(code, undefined))),
            ScardCall::TransmitCall(_) => complete_rpce(req, Box::new(TransmitReturn::new(code, None, Vec::new()))),
            ScardCall::StatusCall(_) => complete_rpce(
                req,
                Box::new(StatusReturn::new(
                    code,
                    Vec::new(),
                    CardState::Unknown,
                    undefined,
                    [0u8; 32],
                    0,
                    CharacterSet::Unicode,
                )),
            ),
            ScardCall::StateCall(_) => complete_rpce(
                req,
                Box::new(StateReturn::new(code, CardState::Unknown, undefined, Vec::new())),
            ),
            ScardCall::ControlCall(_) => complete_rpce(req, Box::new(ControlReturn::new(code, Vec::new()))),
            ScardCall::GetAttribCall(_) => complete_rpce(req, Box::new(GetAttribReturn::new(code, Vec::new()))),
            ScardCall::GetTransmitCountCall(_) => complete_rpce(req, Box::new(GetTransmitCountReturn::new(code, 0))),
            ScardCall::GetDeviceTypeIdCall(_) => complete_rpce(req, Box::new(GetDeviceTypeIdReturn::new(code, 0))),
            ScardCall::ReadCacheCall(_) => complete_rpce(req, Box::new(ReadCacheReturn::new(code, Vec::new()))),
            ScardCall::GetReaderIconCall(_) => complete_rpce(req, Box::new(GetReaderIconReturn::new(code, Vec::new()))),
        };

        Ok(vec![message])
    }
}

fn complete_rpce(req: DeviceControlRequest<ScardIoCtlCode>, output: Box<dyn rpce::Encode>) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse::new(
        req,
        NtStatus::SUCCESS,
        Some(output),
    )))
}
