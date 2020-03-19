use ironrdp::{
    dvc::gfx::{
        zgfx, CapabilitiesAdvertisePdu, CapabilitiesV8Flags, CapabilitySet, ClientPdu,
        FrameAcknowledgePdu, QueueDepth, ServerPdu,
    },
    PduParsing,
};
use log::debug;

use super::DynamicChannelDataHandler;
use crate::RdpError;

pub struct Handler {
    decompressor: zgfx::Decompressor,
    decompressed_buffer: Vec<u8>,
    frames_decoded: u32,
}

impl Handler {
    pub fn new() -> Self {
        Self {
            decompressor: zgfx::Decompressor::new(),
            decompressed_buffer: Vec::with_capacity(1024 * 16),
            frames_decoded: 0,
        }
    }
}

impl DynamicChannelDataHandler for Handler {
    fn process_complete_data(
        &mut self,
        complete_data: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, RdpError> {
        self.decompressed_buffer.resize(0, 0);
        self.decompressor
            .decompress(complete_data.as_slice(), &mut self.decompressed_buffer)?;
        let gfx_pdu = ServerPdu::from_buffer(self.decompressed_buffer.as_slice())?;
        debug!("Got GFX PDU: {:?}", gfx_pdu);

        if let ServerPdu::EndFrame(end_frame_pdu) = gfx_pdu {
            self.frames_decoded += 1;
            let client_pdu = ClientPdu::FrameAcknowledge(FrameAcknowledgePdu {
                queue_depth: QueueDepth::Suspend,
                frame_id: end_frame_pdu.frame_id,
                total_frames_decoded: self.frames_decoded,
            });
            debug!("Sending GFX PDU: {:?}", client_pdu);

            let mut client_pdu_buffer = Vec::with_capacity(client_pdu.buffer_length());
            client_pdu.to_buffer(&mut client_pdu_buffer)?;

            Ok(Some(client_pdu_buffer))
        } else {
            Ok(None)
        }
    }
}

pub fn create_capabilities_advertise() -> Result<Vec<u8>, RdpError> {
    let capabilities_advertise =
        ClientPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(vec![CapabilitySet::V8 {
            flags: CapabilitiesV8Flags::empty(),
        }]));
    let mut capabilities_advertise_buffer =
        Vec::with_capacity(capabilities_advertise.buffer_length());
    capabilities_advertise.to_buffer(&mut capabilities_advertise_buffer)?;

    Ok(capabilities_advertise_buffer)
}
