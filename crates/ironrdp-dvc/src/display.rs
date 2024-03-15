use crate::encode_dvc_messages;
use crate::vec;
use crate::Box;
use crate::DvcClientProcessor;
use crate::DvcMessages;
use crate::DvcProcessor;
use crate::PduResult;
use crate::SvcMessage;
use crate::Vec;
use ironrdp_pdu::cursor::WriteCursor;
use ironrdp_pdu::dvc;
use ironrdp_pdu::other_err;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::PduEncode;
use ironrdp_pdu::PduParsing;
use ironrdp_svc::impl_as_any;

pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";

const RDP_DISPLAY_HEADER_SIZE: usize = 8;

pub struct DisplayControlClient {}

impl_as_any!(DisplayControlClient);

impl DvcProcessor for DisplayControlClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<DvcMessages> {
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<DvcMessages> {
        // TODO: We can parse the payload here for completeness sake,
        // in practice we don't need to do anything with the payload.
        debug!("Got Display PDU of length: {}", payload.len());
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for DisplayControlClient {}

impl DisplayControlClient {
    pub fn new() -> Self {
        Self {}
    }

    pub fn encode_monitors(&self, channel_id: u32, monitors: Vec<dvc::display::Monitor>) -> PduResult<Vec<SvcMessage>> {
        let mut buf = WriteBuf::new();
        let pdu = dvc::display::ClientPdu::DisplayControlMonitorLayout(dvc::display::MonitorLayoutPdu { monitors });
        encode_dvc_messages(channel_id, vec![Box::new(pdu)], None)
    }
}

impl Default for DisplayControlClient {
    fn default() -> Self {
        Self::new()
    }
}

// TODO: dvc::display should ultimately be moved into here
