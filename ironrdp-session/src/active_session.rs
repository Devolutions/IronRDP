mod codecs;
mod fast_path;
mod x224;

use bytes::{BufMut as _, BytesMut};
use ironrdp::fast_path::FastPathError;
use ironrdp::{RdpPdu, Rectangle};
use log::warn;

use crate::connection_sequence::ConnectionSequenceResult;
use crate::image::DecodedImage;
use crate::transport::{Decoder, RdpTransport};
use crate::{utils, InputConfig, RdpError};

pub use self::x224::GfxHandler;

pub struct ActiveStageProcessor {
    x224_processor: x224::Processor,
    fast_path_processor: fast_path::Processor,
}

impl ActiveStageProcessor {
    pub fn new(
        config: InputConfig,
        graphics_handler: Option<Box<dyn GfxHandler + Send>>,
        connection_sequence_result: ConnectionSequenceResult,
    ) -> Self {
        let x224_processor = x224::Processor::new(
            utils::swap_hashmap_kv(connection_sequence_result.joined_static_channels),
            config.global_channel_name,
            config.graphics_config,
            graphics_handler,
        );

        let fast_path_processor = fast_path::ProcessorBuilder {
            global_channel_id: connection_sequence_result.global_channel_id,
            initiator_id: connection_sequence_result.initiator_id,
        }
        .build();

        Self {
            x224_processor,
            fast_path_processor,
        }
    }

    pub async fn process(
        &mut self,
        image: &mut DecodedImage,
        frame: BytesMut,
    ) -> Result<Vec<ActiveStageOutput>, RdpError> {
        let mut output_writer = BytesMut::new().writer();
        let mut frame_reader = frame.as_ref();
        let mut graphics_update_region = None;

        match RdpTransport.decode(&mut frame_reader) {
            Ok(RdpPdu::X224(data)) => {
                if let Err(error) = self.x224_processor.process(frame_reader, &mut output_writer, data) {
                    match error {
                        RdpError::UnexpectedDisconnection(message) => {
                            warn!("User-Initiated disconnection on Server: {}", message);
                            return Ok(vec![ActiveStageOutput::Terminate]);
                        }
                        RdpError::UnexpectedChannel(channel_id) => {
                            warn!("Got message on a channel with {} ID", channel_id);
                            return Ok(vec![ActiveStageOutput::Terminate]);
                        }
                        err => {
                            return Err(err);
                        }
                    }
                }
            }
            Ok(RdpPdu::FastPath(header)) => {
                // skip header bytes in such way because here is possible
                // that data length was written in the not right way,
                // so we should skip only what has been actually read

                graphics_update_region =
                    self.fast_path_processor
                        .process(image, &header, frame_reader, &mut output_writer)?;
            }
            Err(RdpError::FastPathError(FastPathError::NullLength { bytes_read: _ })) => {
                warn!("Received null-length Fast-Path packet, dropping it");
            }
            Err(e) => return Err(e),
        }

        let mut stage_outputs = Vec::new();

        let output_buffer = output_writer.into_inner();
        if !output_buffer.is_empty() {
            stage_outputs.push(ActiveStageOutput::ResponseFrame(output_buffer));
        }

        if let Some(update_region) = graphics_update_region {
            stage_outputs.push(ActiveStageOutput::GraphicsUpdate(update_region));
        }

        Ok(stage_outputs)
    }
}

pub enum ActiveStageOutput {
    ResponseFrame(BytesMut),
    GraphicsUpdate(Rectangle),
    Terminate,
}
