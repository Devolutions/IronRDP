use crate::{
    pdu::{DisplayControlCapabilities, DisplayControlPdu},
    CHANNEL_NAME,
};
use ironrdp_dvc::{encode_dvc_messages, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{cursor::ReadCursor, PduDecode, PduResult};
use ironrdp_svc::{impl_as_any, ChannelFlags, SvcMessage};
use tracing::debug;

/// A client for the Display Control Virtual Channel.
pub struct DisplayControlClient {
    /// A callback that will be called when capabilities are received from the server.
    /// If no callback is set, a default (inert) callback will be used.
    on_capabilities_received: OnCapabilitiesReceived,
}

impl Default for DisplayControlClient {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayControlClient {
    pub fn new() -> Self {
        Self {
            on_capabilities_received: Box::new(|_| {
                debug!("No capabilities received callback set, ignoring.");
                Ok(Vec::new())
            }),
        }
    }

    /// Sets a callback that will be called when capabilities are received from the server.
    #[must_use]
    pub fn with_capabilities_received_callback<F>(mut self, callback: F) -> Self
    where
        F: Fn(DisplayControlCapabilities) -> PduResult<Vec<DvcMessage>> + Send + Sync + 'static,
    {
        self.on_capabilities_received = Box::new(callback);
        self
    }

    /// Builds a [`DisplayControlPdu::MonitorLayout`] as an [`SvcMessage`] for a monitor with the given dimensions.
    pub fn encode_monitor(&self, channel_id: u32, width: u32, height: u32) -> PduResult<Vec<SvcMessage>> {
        let pdu = DisplayControlPdu::create_monitor_layout_pdu(width, height)?;
        encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
    }
}

impl_as_any!(DisplayControlClient);

impl DvcProcessor for DisplayControlClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let caps = DisplayControlCapabilities::decode(&mut ReadCursor::new(payload))?;
        debug!("received {:?}", caps);
        (self.on_capabilities_received)(caps)
    }
}

impl DvcClientProcessor for DisplayControlClient {}

type OnCapabilitiesReceived = Box<dyn Fn(DisplayControlCapabilities) -> PduResult<Vec<DvcMessage>> + Send + Sync>;
