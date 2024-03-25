use crate::{
    pdu::{DisplayControlCapabilities, DisplayControlMonitorLayout, DisplayControlPdu, MonitorLayoutEntry},
    CHANNEL_NAME,
};
use ironrdp_dvc::{encode_dvc_messages, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{cursor::ReadCursor, PduDecode, PduResult};
use ironrdp_svc::{impl_as_any, ChannelFlags, SvcMessage};
use tokio::sync::mpsc::{self as tokio_mpsc};
use tracing::{debug, warn};

/// A client for the Display Control Virtual Channel.
pub struct DisplayControlClient {
    ready_notifier: tokio_mpsc::UnboundedSender<()>,
    ready_listener: Option<tokio_mpsc::UnboundedReceiver<()>>,
}

impl_as_any!(DisplayControlClient);

impl DisplayControlClient {
    pub fn new() -> Self {
        let (ready_notifier, ready_listener) = tokio_mpsc::unbounded_channel();
        Self {
            ready_notifier,
            ready_listener: Some(ready_listener),
        }
    }

    /// Returns a channel that receives a message when the ['DisplayControlClient`] channel is ready
    /// to send messages. (In practice this means that the server has sent the capabilities PDU).
    ///
    /// The channel channel can only be taken once. If the channel is already taken, None is returned.
    pub fn take_ready_listener(&mut self) -> Option<tokio_mpsc::UnboundedReceiver<()>> {
        self.ready_listener.take()
    }

    fn notify_ready(&self) {
        if self.ready_notifier.send(()).is_err() {
            warn!("Failed to notify async open listener: channel is closed");
        }
    }

    /// Fully encodes a [`MonitorLayoutPdu`] with the given monitors.
    pub fn encode_monitors(&self, channel_id: u32, monitors: Vec<MonitorLayoutEntry>) -> PduResult<Vec<SvcMessage>> {
        let pdu: DisplayControlPdu = DisplayControlMonitorLayout::new(&monitors)?.into();
        encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
    }
}

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
        self.notify_ready();
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for DisplayControlClient {}

impl Default for DisplayControlClient {
    fn default() -> Self {
        Self::new()
    }
}
