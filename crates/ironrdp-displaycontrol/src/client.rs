use crate::{
    pdu::{DisplayControlMonitorLayout, DisplayControlPdu, MonitorLayoutEntry},
    CHANNEL_NAME,
};
use ironrdp_dvc::{encode_dvc_messages, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::PduResult;
use ironrdp_svc::{impl_as_any, ChannelFlags, SvcMessage};
use tracing::debug;

/// A client for the Display Control Virtual Channel.
pub struct DisplayControlClient;

impl_as_any!(DisplayControlClient);

impl DvcProcessor for DisplayControlClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        // TODO: We can parse the payload here for completeness sake,
        // in practice we don't need to do anything with the payload.
        debug!("Got Display PDU of length: {}", payload.len());
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for DisplayControlClient {}

impl DisplayControlClient {
    pub fn new() -> Self {
        Self
    }

    /// Fully encodes a [`MonitorLayoutPdu`] with the given monitors.
    pub fn encode_monitors(&self, channel_id: u32, monitors: Vec<MonitorLayoutEntry>) -> PduResult<Vec<SvcMessage>> {
        let pdu: DisplayControlPdu = DisplayControlMonitorLayout::new(&monitors)?.into();
        encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
    }
}

impl Default for DisplayControlClient {
    fn default() -> Self {
        Self::new()
    }
}
