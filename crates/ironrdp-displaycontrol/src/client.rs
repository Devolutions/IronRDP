use ironrdp_core::{impl_as_any, Decode, EncodeResult, ReadCursor};
use ironrdp_dvc::{encode_dvc_messages, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{decode_err, PduResult};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::debug;

use crate::pdu::{DisplayControlCapabilities, DisplayControlMonitorLayout, DisplayControlPdu};
use crate::CHANNEL_NAME;

/// A client for the Display Control Virtual Channel.
pub struct DisplayControlClient {
    /// A callback that will be called when capabilities are received from the server.
    on_capabilities_received: OnCapabilitiesReceived,
    /// Indicates whether the capabilities have been received from the server.
    ready: bool,
}

impl DisplayControlClient {
    /// Creates a new [`DisplayControlClient`] with the given `callback`.
    ///
    /// The `callback` will be called when capabilities are received from the server.
    /// It is important to note that the channel will not be fully operational until the capabilities are received.
    /// Attempting to send messages before the capabilities are received will result in an error or a silent failure.
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(DisplayControlCapabilities) -> PduResult<Vec<DvcMessage>> + Send + 'static,
    {
        Self {
            on_capabilities_received: Box::new(callback),
            ready: false,
        }
    }

    pub fn ready(&self) -> bool {
        self.ready
    }

    /// Builds a [`DisplayControlPdu::MonitorLayout`] with a single primary monitor
    /// with the given `width` and `height`, and wraps it as an [`SvcMessage`].
    ///
    /// Per [2.2.2.2.1]:
    /// - The `width` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels, and MUST NOT be an odd value.
    /// - The `height` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels.
    /// - The `scale_factor` MUST be ignored if it is less than 100 percent or greater than 500 percent.
    /// - The `physical_dims` (width, height) MUST be ignored if either is less than 10 mm or greater than 10,000 mm.
    ///
    /// Use [`crate::pdu::MonitorLayoutEntry::adjust_display_size`] to adjust `width` and `height` before calling this function
    /// to ensure the display size is within the valid range.
    ///
    /// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    pub fn encode_single_primary_monitor(
        &self,
        channel_id: u32,
        width: u32,
        height: u32,
        scale_factor: Option<u32>,
        physical_dims: Option<(u32, u32)>,
    ) -> EncodeResult<Vec<SvcMessage>> {
        // TODO: prevent resolution with values greater than max monitor area received in caps.
        let pdu: DisplayControlPdu =
            DisplayControlMonitorLayout::new_single_primary_monitor(width, height, scale_factor, physical_dims)?.into();
        debug!(?pdu, "Sending monitor layout");
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
        let caps = DisplayControlCapabilities::decode(&mut ReadCursor::new(payload)).map_err(|e| decode_err!(e))?;
        debug!("Received {:?}", caps);
        self.ready = true;
        (self.on_capabilities_received)(caps)
    }
}

impl DvcClientProcessor for DisplayControlClient {}

type OnCapabilitiesReceived = Box<dyn Fn(DisplayControlCapabilities) -> PduResult<Vec<DvcMessage>> + Send>;
