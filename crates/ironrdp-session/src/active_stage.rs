use ironrdp_connector::ConnectionResult;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::Action;

use crate::image::DecodedImage;
use crate::x224::GfxHandler;
use crate::{fast_path, utils, x224, SessionResult};

pub struct ActiveStage {
    x224_processor: x224::Processor,
    fast_path_processor: fast_path::Processor,
}

impl ActiveStage {
    pub fn new(connection_result: ConnectionResult, graphics_handler: Option<Box<dyn GfxHandler + Send>>) -> Self {
        let x224_processor = x224::Processor::new(
            utils::swap_hashmap_kv(connection_result.static_channels),
            connection_result.user_channel_id,
            connection_result.io_channel_id,
            connection_result.graphics_config,
            graphics_handler,
        );

        let fast_path_processor = fast_path::ProcessorBuilder {
            io_channel_id: connection_result.io_channel_id,
            user_channel_id: connection_result.user_channel_id,
        }
        .build();

        Self {
            x224_processor,
            fast_path_processor,
        }
    }

    pub fn process(
        &mut self,
        image: &mut DecodedImage,
        action: Action,
        frame: &[u8],
    ) -> SessionResult<Vec<ActiveStageOutput>> {
        let mut graphics_update_region = None;

        let output = match action {
            Action::FastPath => {
                let mut output = Vec::new();
                graphics_update_region = self.fast_path_processor.process(image, frame, &mut output)?;
                output
            }
            Action::X224 => self.x224_processor.process(frame)?,
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

    /// Sends a PDU on the dynamic channel.
    pub fn encode_dynamic(&self, output: &mut Vec<u8>, channel_name: &str, dvc_data: &[u8]) -> SessionResult<usize> {
        self.x224_processor.encode_dynamic(output, channel_name, dvc_data)
    }

    /// Send a pdu on the static global channel. Typically used to send input events
    pub fn encode_static(
        &self,
        output: &mut Vec<u8>,
        pdu: ironrdp_pdu::rdp::headers::ShareDataPdu,
    ) -> SessionResult<usize> {
        self.x224_processor.encode_static(output, pdu)
    }
}

pub enum ActiveStageOutput {
    ResponseFrame(Vec<u8>),
    GraphicsUpdate(InclusiveRectangle),
    Terminate,
}
