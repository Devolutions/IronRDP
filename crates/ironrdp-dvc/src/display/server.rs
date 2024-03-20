use crate::vec;
use crate::Box;
use crate::DvcServerProcessor;
use ironrdp_pdu::decode;
use ironrdp_pdu::PduResult;
use ironrdp_svc::impl_as_any;

use crate::{DvcMessages, DvcProcessor};

use super::{DisplayControlCapsPdu, DisplayControlPdu, CHANNEL_NAME};

/// A server for the Display Control Virtual Channel.
pub struct DisplayControlServer {}

impl_as_any!(DisplayControlServer);

impl DvcProcessor for DisplayControlServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<DvcMessages> {
        let pdu: DisplayControlPdu = DisplayControlCapsPdu::new(1, 3840, 2400).into();

        Ok(vec![Box::new(pdu)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<DvcMessages> {
        match decode(payload)? {
            DisplayControlPdu::MonitorLayout(layout) => {
                debug!(?layout);
            }
            DisplayControlPdu::Caps(caps) => {
                debug!(?caps);
            }
        }
        Ok(vec![])
    }
}

impl DvcServerProcessor for DisplayControlServer {}
