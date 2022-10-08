use ironrdp::{dvc::display::ServerPdu, PduParsing};
use log::debug;

use super::DynamicChannelDataHandler;
use crate::RdpError;

pub struct Handler {}

impl Handler {
    pub fn new() -> Self {
        Self {}
    }
}

impl DynamicChannelDataHandler for Handler {
    fn process_complete_data(&mut self, complete_data: Vec<u8>) -> Result<Option<Vec<u8>>, RdpError> {
        let gfx_pdu = ServerPdu::from_buffer(&mut complete_data.as_slice())?;
        debug!("Got Display PDU: {:?}", gfx_pdu);
        Ok(None)
    }
}
