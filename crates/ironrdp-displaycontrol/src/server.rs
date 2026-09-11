use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{PduResult, decode_err};
use tracing::debug;

use crate::CHANNEL_NAME;
use crate::pdu::{DisplayControlCapabilities, DisplayControlMonitorLayout, DisplayControlPdu};

pub trait DisplayControlHandler: Send {
    fn monitor_layout(&self, layout: DisplayControlMonitorLayout) {
        debug!(?layout);
    }

    /// Capabilities advertised to the client when the channel starts.
    ///
    /// Defaults to a single-monitor value (`max_num_monitors = 1`, factors
    /// `3840`/`2400`, i.e. one 4K-area monitor), so any handler that doesn't
    /// override this keeps that behavior.
    /// A handler serving more than one monitor should override this, e.g.
    /// `DisplayControlCapabilities::new(monitor_count, 3840, 2400)`. Fallible
    /// because [`DisplayControlCapabilities::new`] validates its arguments
    /// (MS-RDPEDISP does not bound them, but the wire encoding does); an
    /// overrider deriving values from runtime display state should propagate
    /// that error rather than `expect` it, since this is called from
    /// [`DvcProcessor::start`] and a panic there aborts the connection.
    fn capabilities(&self) -> PduResult<DisplayControlCapabilities> {
        DisplayControlCapabilities::new(1, 3840, 2400).map_err(|e| decode_err!(e))
    }
}

/// A server for the Display Control Virtual Channel.
pub struct DisplayControlServer {
    handler: Box<dyn DisplayControlHandler>,
}

impl DisplayControlServer {
    /// Create a new DisplayControlServer.
    pub fn new(handler: Box<dyn DisplayControlHandler>) -> Self {
        Self { handler }
    }
}

impl_as_any!(DisplayControlServer);

impl DvcProcessor for DisplayControlServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let pdu: DisplayControlPdu = self.handler.capabilities()?.into();

        Ok(vec![Box::new(pdu)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode(payload).map_err(|e| decode_err!(e))? {
            DisplayControlPdu::MonitorLayout(layout) => self.handler.monitor_layout(layout),
            DisplayControlPdu::Caps(caps) => {
                debug!(?caps);
            }
        }
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for DisplayControlServer {}
