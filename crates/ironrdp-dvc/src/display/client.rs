use super::{DisplayControlPdu, Monitor, MonitorLayoutPdu, CHANNEL_NAME};
use crate::{encode_dvc_messages, vec, Box, DvcClientProcessor, Vec};
use crate::{DvcMessages, DvcProcessor};
use ironrdp_pdu::{write_buf::WriteBuf, PduResult};
use ironrdp_svc::{impl_as_any, SvcMessage};

/// A client for the Display Control Virtual Channel.
pub struct DisplayControlClient {}

impl_as_any!(DisplayControlClient);

impl DvcProcessor for DisplayControlClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<DvcMessages> {
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<DvcMessages> {
        // TODO: We can parse the payload here for completeness sake,
        // in practice we don't need to do anything with the payload.
        debug!("Got Display PDU of length: {}", payload.len());
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for DisplayControlClient {}

impl DisplayControlClient {
    pub fn new() -> Self {
        Self {}
    }

    /// Fully encodes a [`MonitorLayoutPdu`] with the given monitors.
    pub fn encode_monitors(&self, channel_id: u32, monitors: Vec<Monitor>) -> PduResult<Vec<SvcMessage>> {
        let mut buf = WriteBuf::new();
        let pdu: DisplayControlPdu = MonitorLayoutPdu::new(monitors).into();
        encode_dvc_messages(channel_id, vec![Box::new(pdu)], None)
    }
}

impl Default for DisplayControlClient {
    fn default() -> Self {
        Self::new()
    }
}
