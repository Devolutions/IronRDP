use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{decode, decode_err, PduResult};
use tracing::debug;

use crate::{
    pdu::{DisplayControlCapabilities, DisplayControlMonitorLayout, DisplayControlPdu},
    CHANNEL_NAME,
};

pub trait DisplayControlHandler: Send {
    fn monitor_layout(&self, layout: DisplayControlMonitorLayout) {
        debug!(?layout);
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
        let pdu: DisplayControlPdu = DisplayControlCapabilities::new(1, 3840, 2400)
            .map_err(|e| decode_err!(e))?
            .into();

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
