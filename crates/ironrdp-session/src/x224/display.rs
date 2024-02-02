use ironrdp_pdu::decode;
use ironrdp_pdu::dvc::display::ServerPdu;

use super::DynamicChannelDataHandler;
use crate::{SessionError, SessionErrorExt, SessionResult};

pub(crate) struct Handler;

impl DynamicChannelDataHandler for Handler {
    fn process_complete_data(&mut self, complete_data: Vec<u8>) -> SessionResult<Option<Vec<u8>>> {
        let gfx_pdu: ServerPdu = decode(&complete_data).map_err(SessionError::pdu)?;
        debug!("Got Display PDU: {:?}", gfx_pdu);
        Ok(None)
    }
}
