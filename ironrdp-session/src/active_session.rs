mod codecs;
mod fast_path;
mod x224;

use bytes::{BufMut as _, Bytes, BytesMut};
use ironrdp_core::fast_path::FastPathError;
use ironrdp_core::geometry::Rectangle;
use ironrdp_core::{PduHeader, PduParsing as _};

pub use self::x224::GfxHandler;
use crate::connection_sequence::ConnectionSequenceResult;
use crate::image::DecodedImage;
use crate::{utils, InputConfig, RdpError};

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
            connection_sequence_result.initiator_id,
            connection_sequence_result.global_channel_id,
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

    // TODO: async version?
    /// Sends a PDU on the dynamic channel. The upper layers are responsible for encoding the PDU and converting them to message
    pub fn send_dynamic(
        &mut self,
        stream: impl std::io::Write,
        channel_name: &str,
        message: Bytes,
    ) -> Result<(), RdpError> {
        self.x224_processor.send_dynamic(stream, channel_name, message)
    }

    // TODO: async version?
    /// Send a pdu on the static global channel. Typically used to send input events
    pub fn send_static(
        &self,
        stream: impl std::io::Write,
        message: ironrdp_core::ShareDataPdu,
    ) -> Result<(), RdpError> {
        self.x224_processor.send_static(stream, message)
    }

    pub fn process(&mut self, image: &mut DecodedImage, frame: Bytes) -> Result<Vec<ActiveStageOutput>, RdpError> {
        let mut graphics_update_region = None;

        let output = match PduHeader::from_buffer(&frame[..]).map_err(RdpError::from) {
            Ok(PduHeader::X224(_header)) => match self.x224_processor.process(frame) {
                Ok(output) => output,
                Err(RdpError::UnexpectedDisconnection(message)) => {
                    warn!("User-Initiated disconnection on Server: {}", message);
                    return Ok(vec![ActiveStageOutput::Terminate]);
                }
                Err(RdpError::UnexpectedChannel(channel_id)) => {
                    warn!("Got message on a channel with {} ID", channel_id);
                    return Ok(vec![ActiveStageOutput::Terminate]);
                }
                Err(err) => {
                    return Err(err);
                }
            },
            Ok(PduHeader::FastPath(header)) => {
                let mut output_writer = BytesMut::new().writer();
                graphics_update_region = self.fast_path_processor.process(
                    image,
                    &header,
                    &frame[header.buffer_length()..],
                    &mut output_writer,
                )?;
                output_writer.into_inner()
            }
            Err(RdpError::FastPath(FastPathError::NullLength { bytes_read: _ })) => {
                warn!("Received null-length Fast-Path packet, dropping it");
                BytesMut::new()
            }
            Err(e) => return Err(e),
        };

        let mut stage_outputs = Vec::new();

        if !output.is_empty() {
            stage_outputs.push(ActiveStageOutput::ResponseFrame(output));
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
