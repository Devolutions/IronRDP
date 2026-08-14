//! Stub Windows smartcard session for RDPDR device ID 0.
//!
//! Completes every decoded MS-RDPESC call with the IOCTL-appropriate return
//! PDU and `SCARD_E_UNSUPPORTED_FEATURE`, so peers never hang or decode the
//! wrong structure while the real WinSCard backend lands.

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
/// This stub holds no native WinSCard resources.
/// It provides the `new`, `reset`, `poll`, and `handle_call` hooks expected by [`super::backend::WindowsRdpdrBackend`].
/// `reset` clears queued messages, and `poll` drains them for deferred completions.
#[derive(Debug, Default)]
pub(super) struct ScardSession {
    messages: Vec<SvcMessage>,
}

impl ScardSession {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn reset(&mut self) {
        self.messages.clear();
    }

    pub(super) fn poll(&mut self) -> Vec<SvcMessage> {
        std::mem::take(&mut self.messages)
    }

    pub(super) fn handle_call(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        let output = stub_output(req.io_control_code, call);
        self.messages.push(complete_rpce(req, output));
        Ok(self.poll())
    }
}

fn stub_output(io_control_code: ScardIoCtlCode, call: ScardCall) -> Box<dyn rpce::Encode> {
    let code = ReturnCode::UnsupportedFeature;
    let empty_context = ScardContext::new(0);
    let empty_handle = ScardHandle::new(empty_context, 0);
    let undefined = CardProtocol::SCARD_PROTOCOL_UNDEFINED;
    // Status_Return.mszReaderNames encoding follows the A/W IOCTL form.
    let status_charset = match io_control_code {
        ScardIoCtlCode::StatusA => CharacterSet::Ansi,
        _ => CharacterSet::Unicode,
    };

    match call {
        ScardCall::AccessStartedEventCall(_)
        | ScardCall::HCardAndDispositionCall(_)
        | ScardCall::SetAttribCall(_)
        | ScardCall::ContextCall(_)
        | ScardCall::WriteCacheCall(_)
        | ScardCall::ContextAndStringCall(_)
        | ScardCall::ContextAndTwoStringCall(_) => Box::new(LongReturn::new(code)),
        // MS-RDPESC 3.1.4.46 marks SCARD_IOCTL_RELEASETARTEDEVENT unused with
        // no output packet. The server still issued an IRP, so retire it with
        // Long_Return rather than leaving the IRP unanswered.
        ScardCall::Unsupported => Box::new(LongReturn::new(code)),
        ScardCall::EstablishContextCall(_) => Box::new(EstablishContextReturn::new(code, empty_context)),
        ScardCall::ListReaderGroupsCall(_) | ScardCall::ListReadersCall(_) => {
            Box::new(ListReadersReturn::new(code, Vec::new()))
        }
        ScardCall::GetStatusChangeCall(_) | ScardCall::LocateCardsCall(_) | ScardCall::LocateCardsByAtrCall(_) => {
            Box::new(GetStatusChangeReturn::new(code, Vec::new()))
        }
        ScardCall::ConnectCall(_) => Box::new(ConnectReturn::new(code, empty_handle, undefined)),
        ScardCall::ReconnectCall(_) => Box::new(ReconnectReturn::new(code, undefined)),
        ScardCall::TransmitCall(_) => Box::new(TransmitReturn::new(code, None, Vec::new())),
        ScardCall::StatusCall(_) => Box::new(StatusReturn::new(
            code,
            Vec::new(),
            CardState::Unknown,
            undefined,
            [0u8; 32],
            0,
            status_charset,
        )),
        ScardCall::StateCall(_) => Box::new(StateReturn::new(code, CardState::Unknown, undefined, Vec::new())),
        ScardCall::ControlCall(_) => Box::new(ControlReturn::new(code, Vec::new())),
        ScardCall::GetAttribCall(_) => Box::new(GetAttribReturn::new(code, Vec::new())),
        ScardCall::GetTransmitCountCall(_) => Box::new(GetTransmitCountReturn::new(code, 0)),
        ScardCall::GetDeviceTypeIdCall(_) => Box::new(GetDeviceTypeIdReturn::new(code, 0)),
        ScardCall::ReadCacheCall(_) => Box::new(ReadCacheReturn::new(code, Vec::new())),
        ScardCall::GetReaderIconCall(_) => Box::new(GetReaderIconReturn::new(code, Vec::new())),
    }
}

fn complete_rpce(req: DeviceControlRequest<ScardIoCtlCode>, output: Box<dyn rpce::Encode>) -> SvcMessage {
    let (status, output_buffer) = if output.size() <= req.output_buffer_length as usize {
        (NtStatus::SUCCESS, Some(output))
    } else {
        (NtStatus::BUFFER_TOO_SMALL, None)
    };

    SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse::new(
        req,
        status,
        output_buffer,
    )))
}

#[cfg(test)]
mod tests {
    use ironrdp_rdpdr::pdu::efs::{DeviceIoRequest, MajorFunction, MinorFunction};
    use ironrdp_rdpdr::pdu::esc::{EstablishContextCall, ScardAccessStartedEventCall, Scope};

    use super::*;

    fn establish_context_call() -> ScardCall {
        ScardCall::EstablishContextCall(EstablishContextCall { scope: Scope::User })
    }

    fn control_req(code: ScardIoCtlCode, device_id: u32, completion_id: u32) -> DeviceControlRequest<ScardIoCtlCode> {
        DeviceControlRequest {
            header: DeviceIoRequest {
                device_id,
                file_id: 0,
                completion_id,
                major_function: MajorFunction::DeviceControl,
                minor_function: MinorFunction::from(0),
            },
            output_buffer_length: 2048,
            input_buffer_length: 0,
            io_control_code: code,
        }
    }

    #[test]
    fn establish_context_maps_to_establish_context_return() {
        let output = stub_output(ScardIoCtlCode::EstablishContext, establish_context_call());
        assert_eq!(output.name(), "EstablishContext_Return");
    }

    #[test]
    fn long_return_calls_map_to_long_return() {
        let output = stub_output(
            ScardIoCtlCode::AccessStartedEvent,
            ScardCall::AccessStartedEventCall(ScardAccessStartedEventCall),
        );
        assert_eq!(output.name(), "Long_Return");
    }

    #[test]
    fn unsupported_release_tarted_event_maps_to_long_return() {
        let output = stub_output(ScardIoCtlCode::ReleaseTartedEvent, ScardCall::Unsupported);
        assert_eq!(output.name(), "Long_Return");
    }

    #[test]
    fn connect_and_list_readers_map_to_table_return_names() {
        // Call payloads are unused by the stub mapping; only the enum arm matters.
        let connect = stub_output(
            ScardIoCtlCode::ConnectW,
            ScardCall::ConnectCall(ironrdp_rdpdr::pdu::esc::ConnectCall {
                reader: String::new(),
                common: ironrdp_rdpdr::pdu::esc::ConnectCommon {
                    context: ScardContext::new(0),
                    share_mode: 0,
                    preferred_protocols: CardProtocol::SCARD_PROTOCOL_UNDEFINED,
                },
            }),
        );
        assert_eq!(connect.name(), "Connect_Return");

        let readers = stub_output(
            ScardIoCtlCode::ListReadersW,
            ScardCall::ListReadersCall(ironrdp_rdpdr::pdu::esc::ListReadersCall {
                context: ScardContext::new(0),
                groups_ptr_length: 0,
                groups_length: 0,
                groups_ptr: 0,
                groups: Vec::new(),
                readers_is_null: true,
                readers_size: 0,
            }),
        );
        assert_eq!(readers.name(), "ListReaders_Return");
    }

    #[test]
    fn status_a_and_status_w_select_matching_charset_encoding() {
        fn encode(output: &dyn rpce::Encode) -> Vec<u8> {
            ironrdp_core::encode_vec(output).expect("encode Status_Return")
        }

        let status_call = ScardCall::StatusCall(ironrdp_rdpdr::pdu::esc::StatusCall {
            handle: ScardHandle::new(ScardContext::new(0), 0),
            reader_names_is_null: true,
            reader_length: 0,
            atr_length: 0,
        });
        let stub_a = stub_output(ScardIoCtlCode::StatusA, status_call.clone());
        let stub_w = stub_output(ScardIoCtlCode::StatusW, status_call);
        assert_eq!(stub_a.name(), "Status_Return");
        assert_eq!(stub_w.name(), "Status_Return");

        let expected_a = StatusReturn::new(
            ReturnCode::UnsupportedFeature,
            Vec::new(),
            CardState::Unknown,
            CardProtocol::SCARD_PROTOCOL_UNDEFINED,
            [0u8; 32],
            0,
            CharacterSet::Ansi,
        );
        let expected_w = StatusReturn::new(
            ReturnCode::UnsupportedFeature,
            Vec::new(),
            CardState::Unknown,
            CardProtocol::SCARD_PROTOCOL_UNDEFINED,
            [0u8; 32],
            0,
            CharacterSet::Unicode,
        );

        assert_eq!(encode(stub_a.as_ref()), encode(&expected_a), "StatusA -> Ansi");
        assert_eq!(encode(stub_w.as_ref()), encode(&expected_w), "StatusW -> Unicode");
        assert_ne!(
            encode(&expected_a),
            encode(&expected_w),
            "A/W empty multistring encodings must differ"
        );
    }

    #[test]
    fn handle_call_echoes_device_and_completion_ids() {
        let mut session = ScardSession::new();
        let req = control_req(ScardIoCtlCode::EstablishContext, 0, 42);
        let messages = session
            .handle_call(req.clone(), establish_context_call())
            .expect("stub must complete");
        assert_eq!(messages.len(), 1);

        // Rebuild the same response shape and compare encoded wire bytes.
        let expected = complete_rpce(
            req,
            Box::new(EstablishContextReturn::new(
                ReturnCode::UnsupportedFeature,
                ScardContext::new(0),
            )),
        );
        let actual = messages[0].encode_unframed_pdu().expect("encode actual");
        let expected = expected.encode_unframed_pdu().expect("encode expected");
        assert_eq!(actual, expected);
    }

    #[test]
    fn handle_call_respects_output_buffer_length() {
        let mut session = ScardSession::new();
        let req = control_req(ScardIoCtlCode::EstablishContext, 0, 42);
        let messages = session
            .handle_call(
                DeviceControlRequest {
                    output_buffer_length: 0,
                    ..req.clone()
                },
                establish_context_call(),
            )
            .expect("stub must complete");

        let expected = SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse::new(
            req,
            NtStatus::BUFFER_TOO_SMALL,
            None,
        )));
        let actual = messages[0].encode_unframed_pdu().expect("encode actual");
        let expected = expected.encode_unframed_pdu().expect("encode expected");
        assert_eq!(actual, expected);
    }
}
