use bitflags::bitflags;
use ironrdp_core::dvc::gfx::{
    CapabilitiesAdvertisePdu, CapabilitiesV103Flags, CapabilitiesV104Flags, CapabilitiesV107Flags,
    CapabilitiesV10Flags, CapabilitiesV81Flags, CapabilitiesV8Flags, CapabilitySet, ClientPdu, FrameAcknowledgePdu,
    QueueDepth, ServerPdu,
};
use ironrdp_core::PduParsing;
use ironrdp_graphics::zgfx;
use log::debug;

use super::DynamicChannelDataHandler;
use crate::{GraphicsConfig, RdpError};

pub trait GfxHandler {
    fn on_message(&self, message: ServerPdu) -> Result<Option<ClientPdu>, RdpError>;
}

pub struct Handler {
    decompressor: zgfx::Decompressor,
    decompressed_buffer: Vec<u8>,
    frames_decoded: u32,
    gfx_handler: Option<Box<dyn GfxHandler + Send>>,
}

impl Handler {
    pub fn new(gfx_handler: Option<Box<dyn GfxHandler + Send>>) -> Self {
        Self {
            decompressor: zgfx::Decompressor::new(),
            decompressed_buffer: Vec::with_capacity(1024 * 16),
            frames_decoded: 0,
            gfx_handler,
        }
    }
}

impl DynamicChannelDataHandler for Handler {
    fn process_complete_data(&mut self, complete_data: Vec<u8>) -> Result<Option<Vec<u8>>, RdpError> {
        let mut client_pdu_buffer: Vec<u8> = vec![];
        self.decompressed_buffer.clear();
        self.decompressor
            .decompress(complete_data.as_slice(), &mut self.decompressed_buffer)?;
        let mut slice = &mut self.decompressed_buffer.as_slice();
        while !slice.is_empty() {
            let gfx_pdu = ServerPdu::from_buffer(&mut slice)?;
            debug!("Got GFX PDU: {:?}", gfx_pdu);

            if let ServerPdu::EndFrame(end_frame_pdu) = &gfx_pdu {
                self.frames_decoded += 1;
                // Enqueue an acknowledge for every end frame
                let client_pdu = ClientPdu::FrameAcknowledge(FrameAcknowledgePdu {
                    queue_depth: QueueDepth::Suspend,
                    frame_id: end_frame_pdu.frame_id,
                    total_frames_decoded: self.frames_decoded,
                });
                debug!("Sending GFX PDU: {:?}", client_pdu);
                client_pdu_buffer.reserve(client_pdu_buffer.len() + client_pdu.buffer_length());
                client_pdu.to_buffer(&mut client_pdu_buffer)?;
            } else {
                // Handle the normal PDU
            }

            // If there is a listener send all the data to the listener
            if let Some(handler) = self.gfx_handler.as_mut() {
                // Handle the normal PDU
                let client_pdu = handler.on_message(gfx_pdu)?;

                if let Some(client_pdu) = client_pdu {
                    client_pdu_buffer.reserve(client_pdu_buffer.len() + client_pdu.buffer_length());
                    client_pdu.to_buffer(&mut client_pdu_buffer)?;
                }
            }
        }

        if !client_pdu_buffer.is_empty() {
            return Ok(Some(client_pdu_buffer));
        }

        Ok(None)
    }
}

bitflags! {
    struct CapabilityVersion: u32  {
        const V8        = 1 << 0;
        const V8_1      = 1 << 1;
        const V10       = 1 << 2;
        const V10_1     = 1 << 3;
        const V10_2     = 1 << 4;
        const V10_3     = 1 << 5;
        const V10_4     = 1 << 6;
        const V10_5     = 1 << 7;
        const V10_6     = 1 << 8;
        const V10_6ERR  = 1 << 9;
        const V10_7     = 1 << 10;
    }
}

pub fn create_capabilities_advertise(graphics_config: &Option<GraphicsConfig>) -> Result<Vec<u8>, RdpError> {
    let mut capabilities = vec![];

    if let Some(config) = graphics_config {
        let capability_version = CapabilityVersion::from_bits(config.capabilities)
            .ok_or(RdpError::InvalidCapabilitiesMask(config.capabilities))?;

        if capability_version.contains(CapabilityVersion::V8) {
            let flags = if config.thin_client {
                CapabilitiesV8Flags::THIN_CLIENT
            } else if config.small_cache {
                CapabilitiesV8Flags::SMALL_CACHE
            } else {
                CapabilitiesV8Flags::empty()
            };

            capabilities.push(CapabilitySet::V8 { flags });
        }

        if capability_version.contains(CapabilityVersion::V8_1) {
            let mut flags = CapabilitiesV81Flags::empty();
            if config.thin_client {
                flags |= CapabilitiesV81Flags::THIN_CLIENT;
            }

            if config.small_cache {
                flags |= CapabilitiesV81Flags::SMALL_CACHE;
            }

            if config.h264 {
                flags |= CapabilitiesV81Flags::AVC420_ENABLED;
            }

            capabilities.push(CapabilitySet::V8_1 { flags });
        }

        if config.avc444 {
            let flags = if config.small_cache {
                CapabilitiesV10Flags::SMALL_CACHE
            } else {
                CapabilitiesV10Flags::empty()
            };

            if capability_version.contains(CapabilityVersion::V10) {
                capabilities.push(CapabilitySet::V10 { flags });
            }

            if capability_version.contains(CapabilityVersion::V10_1) {
                capabilities.push(CapabilitySet::V10_1 {});
            }

            if capability_version.contains(CapabilityVersion::V10_2) {
                capabilities.push(CapabilitySet::V10_2 { flags });
            }

            if capability_version.contains(CapabilityVersion::V10_3) {
                let flags = if config.thin_client {
                    CapabilitiesV103Flags::AVC_THIN_CLIENT
                } else {
                    CapabilitiesV103Flags::empty()
                };
                capabilities.push(CapabilitySet::V10_3 { flags });
            }

            let mut flags = if config.small_cache {
                CapabilitiesV104Flags::SMALL_CACHE
            } else {
                CapabilitiesV104Flags::empty()
            };

            if config.thin_client {
                flags |= CapabilitiesV104Flags::AVC_THIN_CLIENT;
            }

            if capability_version.contains(CapabilityVersion::V10_4) {
                capabilities.push(CapabilitySet::V10_4 { flags });
            }

            if capability_version.contains(CapabilityVersion::V10_5) {
                capabilities.push(CapabilitySet::V10_5 { flags });
            }

            if capability_version.contains(CapabilityVersion::V10_6) {
                capabilities.push(CapabilitySet::V10_6 { flags });
            }

            if capability_version.contains(CapabilityVersion::V10_6ERR) {
                capabilities.push(CapabilitySet::V10_6Err { flags });
            }

            if capability_version.contains(CapabilityVersion::V10_7) {
                capabilities.push(CapabilitySet::V10_7 {
                    flags: CapabilitiesV107Flags::from_bits(flags.bits()).unwrap(),
                });
            }
        }
    }
    log::info!("Capabilities: {:?}", capabilities);
    let capabilities_advertise = ClientPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(capabilities));
    let mut capabilities_advertise_buffer = Vec::with_capacity(capabilities_advertise.buffer_length());
    capabilities_advertise.to_buffer(&mut capabilities_advertise_buffer)?;

    Ok(capabilities_advertise_buffer)
}
