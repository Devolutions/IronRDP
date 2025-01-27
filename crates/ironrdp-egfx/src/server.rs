use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::{decode_err, PduResult};
use tracing::{trace, warn};

use crate::{
    pdu::{CacheImportOfferPdu, CapabilitiesAdvertisePdu, FrameAcknowledgePdu, GfxPdu},
    CHANNEL_NAME,
};

pub trait GraphicsPipelineHandler: Send {
    fn capabilities_advertise(&mut self, pdu: CapabilitiesAdvertisePdu);

    fn frame_acknowledge(&mut self, pdu: FrameAcknowledgePdu) {
        trace!(?pdu);
    }

    fn cache_import_offer(&mut self, pdu: CacheImportOfferPdu) {
        trace!(?pdu);
    }
}

/// A server for the Display Control Virtual Channel.
pub struct GraphicsPipelineServer {
    handler: Box<dyn GraphicsPipelineHandler>,
}

impl GraphicsPipelineServer {
    /// Create a new GraphicsPipelineServer.
    pub fn new(handler: Box<dyn GraphicsPipelineHandler>) -> Self {
        Self { handler }
    }
}

impl_as_any!(GraphicsPipelineServer);

impl DvcProcessor for GraphicsPipelineServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(vec![])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu = decode(payload).map_err(|e| decode_err!(e))?;
        match pdu {
            GfxPdu::CapabilitiesAdvertise(pdu) => {
                self.handler.capabilities_advertise(pdu);
            }
            GfxPdu::FrameAcknowledge(pdu) => {
                self.handler.frame_acknowledge(pdu);
            }
            GfxPdu::CacheImportOffer(pdu) => {
                self.handler.cache_import_offer(pdu);
            }
            _ => {
                warn!(?pdu, "Unhandled client GFX PDU");
            }
        }
        Ok(vec![])
    }
}

impl DvcServerProcessor for GraphicsPipelineServer {}
