use ironrdp_pdu::dvc::display::ServerPdu;
use ironrdp_pdu::PduParsing;

use crate::SessionResult;

pub(crate) struct Handler;

impl Handler {
    fn process_complete_data(&mut self, complete_data: Vec<u8>) -> SessionResult<Option<Vec<u8>>> {
        let gfx_pdu = ServerPdu::from_buffer(&mut complete_data.as_slice())?;
        debug!("Got Display PDU: {:?}", gfx_pdu);
        Ok(None)
    }
}
