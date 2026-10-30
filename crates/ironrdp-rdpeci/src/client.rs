//! Client processor for the MS-RDPECI dynamic virtual channel.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{CoreInputPdu, InitRequestPdu, InputPdu, ProtocolVersion, decode_received};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    WaitingInitResponse,
    Ready,
    /// The server selected a version this client does not implement, so input stays on fast-path.
    Unsupported,
}

/// Client processor for `Microsoft::Windows::RDS::CoreInput`.
///
/// The processor sends the init request when the channel opens and becomes ready once the server selects version 1.0,
/// per [MS-RDPECI] section 3.3.3 "Initialization".
/// Until then, and when the channel is absent, the caller keeps sending input as fast-path input.
///
/// [MS-RDPECI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeci/
#[derive(Debug)]
pub struct CoreInputClient {
    state: State,
}

impl CoreInputClient {
    pub fn new() -> Self {
        Self {
            state: State::WaitingInitResponse,
        }
    }

    /// Returns whether the version exchange has completed and the channel accepts input.
    pub fn ready(&self) -> bool {
        self.state == State::Ready
    }

    /// Encodes input events for the channel with the given dynamic channel ID.
    ///
    /// The events are split into PDUs of at most [`InputPdu::MAX_EVENTS`] events, in order.
    /// Fails when the channel is not ready.
    pub fn encode_input(&self, channel_id: u32, events: &[FastPathInputEvent]) -> PduResult<Vec<SvcMessage>> {
        if !self.ready() {
            return Err(pdu_other_err!(
                "CoreInputClient::encode_input",
                "core input channel is not ready"
            ));
        }
        let messages = events
            .chunks(InputPdu::MAX_EVENTS)
            .map(|chunk| {
                let pdu = InputPdu::new(chunk.to_vec()).map_err(|error| encode_err!(error))?;
                let message: DvcMessage = Box::new(CoreInputPdu::Input(pdu));
                Ok(message)
            })
            .collect::<PduResult<Vec<_>>>()?;
        encode_dvc_messages(channel_id, messages, ChannelFlags::empty()).map_err(|error| encode_err!(error))
    }
}

impl Default for CoreInputClient {
    fn default() -> Self {
        Self::new()
    }
}

impl_as_any!(CoreInputClient);

impl DvcProcessor for CoreInputClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.state = State::WaitingInitResponse;
        debug!(channel_id, "Core input channel started");
        // [MS-RDPECI] section 3.3.3: the client sends the init request as soon as the channel opens.
        Ok(vec![Box::new(CoreInputPdu::InitRequest(InitRequestPdu {
            protocol_version_min: ProtocolVersion::V1_0,
            protocol_version_max: ProtocolVersion::V1_0,
        }))])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let Some(pdu) = decode_received(payload) else {
            return Ok(Vec::new());
        };
        match pdu {
            CoreInputPdu::InitResponse(response) if self.state == State::WaitingInitResponse => {
                if response.selected_protocol_version == ProtocolVersion::V1_0 {
                    debug!(max_version = ?response.protocol_version_max, "Core input channel ready");
                    self.state = State::Ready;
                } else {
                    warn!(
                        selected_version = ?response.selected_protocol_version,
                        "Core input server selected an unsupported version; input stays on fast-path"
                    );
                    self.state = State::Unsupported;
                }
            }
            CoreInputPdu::InitResponse(_) => warn!("Ignoring out-of-sequence core input init response"),
            CoreInputPdu::InitRequest(_) | CoreInputPdu::Input(_) => {
                warn!("Ignoring client-originated core input PDU received from server");
            }
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.state = State::WaitingInitResponse;
        debug!("Core input channel closed");
    }
}

impl DvcClientProcessor for CoreInputClient {}
