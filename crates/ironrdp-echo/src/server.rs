use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{decode_err, pdu_other_err, PduResult};
use tracing::debug;

use crate::pdu::{EchoRequestPdu, EchoResponsePdu};
use crate::CHANNEL_NAME;

/// A server for the ECHO virtual channel.
#[derive(Debug, Default)]
pub struct EchoServer {
    initial_request: Option<Vec<u8>>,
}

impl EchoServer {
    /// Creates a new [`EchoServer`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures an initial request that will be sent once the ECHO channel is opened.
    #[must_use]
    pub fn with_initial_request(mut self, payload: Vec<u8>) -> Self {
        self.initial_request = Some(payload);
        self
    }

    /// Builds a request message.
    pub fn request_message(payload: Vec<u8>) -> PduResult<DvcMessage> {
        if payload.is_empty() {
            return Err(pdu_other_err!(
                "EchoServer::request_message",
                "echoRequest payload must be at least one byte"
            ));
        }

        Ok(Box::new(EchoRequestPdu::new(payload)))
    }
}

impl_as_any!(EchoServer);

impl DvcProcessor for EchoServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        if let Some(payload) = self.initial_request.take() {
            return Ok(vec![Self::request_message(payload)?]);
        }

        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let response: EchoResponsePdu = decode(payload).map_err(|e| decode_err!(e))?;
        debug!(size = response.payload().len(), "Received ECHO response");
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for EchoServer {}
