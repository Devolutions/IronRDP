use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{PduResult, decode_err};
use tracing::debug;

use crate::CHANNEL_NAME;
use crate::pdu::{DisplayControlCapabilities, DisplayControlMonitorLayout, DisplayControlPdu};

/// A server for the Display Control Virtual Channel.
///
/// The server advertises the capabilities it was created with when the channel starts
/// ([\[MS-RDPEDISP\] 3.1.5.1 Sending DISPLAYCONTROL_CAPS_PDU]).
/// Monitor layouts received from the client are kept until taken with
/// [`Self::take_monitor_layouts`], so the caller applies them
/// ([\[MS-RDPEDISP\] 3.1.5.2 Processing DISPLAYCONTROL_MONITOR_LAYOUT_PDU]).
///
/// [\[MS-RDPEDISP\] 3.1.5.1 Sending DISPLAYCONTROL_CAPS_PDU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/102ce2ee-81e3-476a-83ae-07987d8aeff8
/// [\[MS-RDPEDISP\] 3.1.5.2 Processing DISPLAYCONTROL_MONITOR_LAYOUT_PDU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/32525e02-fc5c-4b82-ae87-88a458444902
pub struct DisplayControlServer {
    capabilities: DisplayControlCapabilities,
    monitor_layouts: Vec<DisplayControlMonitorLayout>,
}

impl DisplayControlServer {
    /// Creates a server that advertises `capabilities` to the client.
    pub fn new(capabilities: DisplayControlCapabilities) -> Self {
        Self {
            capabilities,
            monitor_layouts: Vec::new(),
        }
    }

    /// Takes the monitor layouts received since the last call, in arrival order.
    ///
    /// Each layout is the complete layout the client requested.
    /// Layouts accumulate until taken, so a caller that processes client data must call this.
    pub fn take_monitor_layouts(&mut self) -> Vec<DisplayControlMonitorLayout> {
        core::mem::take(&mut self.monitor_layouts)
    }
}

impl_as_any!(DisplayControlServer);

impl DvcProcessor for DisplayControlServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let pdu: DisplayControlPdu = self.capabilities.into();

        Ok(vec![Box::new(pdu)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode(payload).map_err(|e| decode_err!(e))? {
            DisplayControlPdu::MonitorLayout(layout) => {
                debug!(?layout, "Received monitor layout");
                self.monitor_layouts.push(layout);
            }
            DisplayControlPdu::Caps(caps) => {
                debug!(?caps);
            }
        }
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for DisplayControlServer {}
