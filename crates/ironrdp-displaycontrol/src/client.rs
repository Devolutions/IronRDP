use ironrdp_core::{EncodeResult, decode, impl_as_any, invalid_field_err};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::debug;

use crate::CHANNEL_NAME;
use crate::pdu::{DisplayControlCapabilities, DisplayControlMonitorLayout, DisplayControlPdu};

/// A client for the Display Control Virtual Channel.
pub struct DisplayControlClient {
    /// A callback that will be called when capabilities are received from the server.
    on_capabilities_received: OnCapabilitiesReceived,
    /// The capabilities received from the server, if any.
    capabilities: Option<DisplayControlCapabilities>,
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
            capabilities: None,
        }
    }

    pub fn ready(&self) -> bool {
        self.capabilities.is_some()
    }

    /// Returns the capabilities received from the server, if any.
    pub fn capabilities(&self) -> Option<&DisplayControlCapabilities> {
        self.capabilities.as_ref()
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
    /// See [`DisplayControlMonitorLayout::new_single_primary_monitor`] for how the scale factors and
    /// the orientation are derived. Once capabilities are received, the monitor is scaled down to
    /// the server's maximum monitor area if needed.
    ///
    /// Use [`crate::pdu::MonitorLayoutEntry::adjust_display_size`] to adjust `width` and `height` before calling this function
    /// to ensure the display size is within the valid range.
    ///
    /// [2.2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    pub fn encode_single_primary_monitor(
        &self,
        channel_id: u32,
        width: u32,
        height: u32,
        scale_factor: Option<u32>,
        physical_dims: Option<(u32, u32)>,
    ) -> EncodeResult<Vec<SvcMessage>> {
        let layout =
            DisplayControlMonitorLayout::new_single_primary_monitor(width, height, scale_factor, physical_dims)?;
        let layout = match &self.capabilities {
            Some(capabilities) => layout.fit_to_capabilities(capabilities)?,
            None => layout,
        };
        let pdu = DisplayControlPdu::from(layout);
        debug!(?pdu, "Sending monitor layout");
        encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
    }

    /// Builds a [`DisplayControlPdu::MonitorLayout`] carrying the complete client monitor `layout`,
    /// and wraps it as an [`SvcMessage`].
    ///
    /// The layout is first adapted to the server capabilities with
    /// [`DisplayControlMonitorLayout::fit_to_capabilities`], per [3.2.5.2]. Fails when the
    /// capabilities have not been received yet.
    ///
    /// [3.2.5.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/d2954508-f487-48bc-8731-39743e0854a9
    pub fn encode_monitor_layout(
        &self,
        channel_id: u32,
        layout: &DisplayControlMonitorLayout,
    ) -> EncodeResult<Vec<SvcMessage>> {
        let Some(capabilities) = &self.capabilities else {
            return Err(invalid_field_err!(
                "Capabilities",
                "display control capabilities have not been received"
            ));
        };
        let pdu = DisplayControlPdu::from(layout.fit_to_capabilities(capabilities)?);
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
        // The server sends the full DISPLAYCONTROL_CAPS_PDU, i.e. the capability set prefixed
        // with the DISPLAYCONTROL_HEADER (Type + Length), per MS-RDPEDISP 2.2.2.1.
        let caps = match decode(payload).map_err(|e| decode_err!(e))? {
            DisplayControlPdu::Caps(caps) => caps,
            DisplayControlPdu::MonitorLayout(_) => {
                return Err(pdu_other_err!(
                    "DisplayControlClient::process",
                    "received unexpected DISPLAYCONTROL_MONITOR_LAYOUT_PDU from server"
                ));
            }
        };
        debug!("Received {:?}", caps);
        self.capabilities = Some(caps.clone());
        (self.on_capabilities_received)(caps)
    }
}

impl DvcClientProcessor for DisplayControlClient {}

type OnCapabilitiesReceived = Box<dyn Fn(DisplayControlCapabilities) -> PduResult<Vec<DvcMessage>> + Send>;
