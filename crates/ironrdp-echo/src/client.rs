use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{decode_err, PduResult};
use tracing::debug;

use crate::pdu::{EchoRequestPdu, EchoResponsePdu};
use crate::CHANNEL_NAME;

/// A client for the ECHO virtual channel.
#[derive(Debug, Default)]
pub struct EchoClient;

impl EchoClient {
    /// Creates a new [`EchoClient`].
    pub fn new() -> Self {
        Self
    }
}

impl_as_any!(EchoClient);

impl DvcProcessor for EchoClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let request: EchoRequestPdu = decode(payload).map_err(|e| decode_err!(e))?;
        debug!(size = request.payload().len(), "Received ECHO request");

        let response = EchoResponsePdu::new(request.into_payload());
        Ok(vec![Box::new(response)])
    }
}

impl DvcClientProcessor for EchoClient {}
