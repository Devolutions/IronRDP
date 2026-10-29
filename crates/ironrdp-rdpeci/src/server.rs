//! Server processor for the MS-RDPECI dynamic virtual channel.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::PduResult;
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use tracing::{debug, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{CoreInputPdu, InitResponsePdu, ProtocolVersion};

/// Receives the input events a client sends on the core input channel.
pub trait CoreInputHandler: Send {
    /// Called with the events of each input PDU, in the order the client sent them.
    fn input(&mut self, events: Vec<FastPathInputEvent>);
}

/// Server processor for `Microsoft::Windows::RDS::CoreInput`.
///
/// The processor answers an init request whose version range includes 1.0 and then passes input events to its handler.
/// Input received before that exchange is ignored.
pub struct CoreInputServer {
    handler: Box<dyn CoreInputHandler>,
    ready: bool,
}

impl CoreInputServer {
    pub fn new(handler: Box<dyn CoreInputHandler>) -> Self {
        Self { handler, ready: false }
    }
}

impl core::fmt::Debug for CoreInputServer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CoreInputServer")
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}

impl_as_any!(CoreInputServer);

impl DvcProcessor for CoreInputServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.ready = false;
        debug!(channel_id, "Core input channel started");
        // MS-RDPECI 3.3.3: the client opens the exchange.
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        // MS-RDPECI 3.1.5.1: unexpected messages SHOULD be ignored.
        let pdu: CoreInputPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring malformed core input PDU");
                return Ok(Vec::new());
            }
        };
        match pdu {
            CoreInputPdu::InitRequest(request) => {
                let supported = request.protocol_version_min <= ProtocolVersion::V1_0
                    && ProtocolVersion::V1_0 <= request.protocol_version_max;
                if !supported {
                    warn!(
                        min_version = ?request.protocol_version_min,
                        max_version = ?request.protocol_version_max,
                        "Core input client supports no common version"
                    );
                    return Ok(Vec::new());
                }
                debug!("Core input channel ready");
                self.ready = true;
                return Ok(vec![Box::new(CoreInputPdu::InitResponse(InitResponsePdu {
                    selected_protocol_version: ProtocolVersion::V1_0,
                    protocol_version_max: ProtocolVersion::V1_0,
                }))]);
            }
            CoreInputPdu::Input(input) if self.ready => self.handler.input(input.into_events()),
            CoreInputPdu::Input(_) => warn!("Ignoring core input events received before the init request"),
            CoreInputPdu::InitResponse(_) => {
                warn!("Ignoring server-originated core input PDU received from client");
            }
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.ready = false;
        debug!("Core input channel closed");
    }
}

impl DvcServerProcessor for CoreInputServer {}
