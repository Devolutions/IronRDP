use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{decode, PduResult};
use ironrdp_svc::impl_as_any;
use tracing::debug;

use crate::{
    pdu::{DisplayControlCapabilities, DisplayControlPdu},
    CHANNEL_NAME,
};

/// A server for the Display Control Virtual Channel.
pub struct DisplayControlServer;

impl_as_any!(DisplayControlServer);

impl DvcProcessor for DisplayControlServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let pdu: DisplayControlPdu = DisplayControlCapabilities::new(1, 3840, 2400)?.into();

        Ok(vec![Box::new(pdu)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode(payload)? {
            DisplayControlPdu::MonitorLayout(layout) => {
                debug!(?layout);
            }
            DisplayControlPdu::Caps(caps) => {
                debug!(?caps);
            }
        }
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for DisplayControlServer {}
