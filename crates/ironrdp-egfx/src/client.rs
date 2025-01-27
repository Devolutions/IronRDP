use ironrdp_core::{impl_as_any, ReadCursor};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_graphics::zgfx;
use ironrdp_pdu::{decode_cursor, decode_err, PduResult};
use tracing::trace;

use crate::{
    pdu::{CapabilitiesAdvertisePdu, CapabilitiesV8Flags, CapabilitySet, GfxPdu},
    CHANNEL_NAME,
};

pub trait GraphicsPipelineHandler: Send {
    fn capabilities(&self) -> Vec<CapabilitySet> {
        vec![CapabilitySet::V8 {
            flags: CapabilitiesV8Flags::empty(),
        }]
    }

    fn handle_pdu(&mut self, pdu: GfxPdu) {
        trace!(?pdu);
    }
}

/// A client for the Graphics Pipeline Virtual Channel.
pub struct GraphicsPipelineClient {
    handler: Box<dyn GraphicsPipelineHandler>,
    decompressor: zgfx::Decompressor,
    decompressed_buffer: Vec<u8>,
}

impl GraphicsPipelineClient {
    pub fn new(handler: Box<dyn GraphicsPipelineHandler>) -> Self {
        Self {
            handler,
            decompressor: zgfx::Decompressor::new(),
            decompressed_buffer: Vec::with_capacity(1024 * 16),
        }
    }
}

impl_as_any!(GraphicsPipelineClient);

impl DvcProcessor for GraphicsPipelineClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(self.handler.capabilities()));

        Ok(vec![Box::new(pdu)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.decompressed_buffer.clear();
        self.decompressor
            .decompress(payload, &mut self.decompressed_buffer)
            .map_err(|e| decode_err!(e))?;

        let mut cursor = ReadCursor::new(self.decompressed_buffer.as_slice());
        while !cursor.is_empty() {
            let pdu = decode_cursor(&mut cursor).map_err(|e| decode_err!(e))?;
            self.handler.handle_pdu(pdu);
        }

        Ok(vec![])
    }
}

impl DvcClientProcessor for GraphicsPipelineClient {}
