use ironrdp_connector::ConnectionResult;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{Action, PduParsing};

use crate::fast_path::UpdateKind;
use crate::image::DecodedImage;
use crate::x224::GfxHandler;
use crate::{fast_path, x224, SessionResult};

pub struct ActiveStage {
    x224_processor: x224::Processor,
    fast_path_processor: fast_path::Processor,
    no_server_pointer: bool,
}

impl ActiveStage {
    pub fn new(connection_result: ConnectionResult, graphics_handler: Option<Box<dyn GfxHandler + Send>>) -> Self {
        let x224_processor = x224::Processor::new(
            connection_result.static_channels,
            connection_result.user_channel_id,
            connection_result.io_channel_id,
            connection_result.graphics_config,
            graphics_handler,
        );

        let fast_path_processor = fast_path::ProcessorBuilder {
            io_channel_id: connection_result.io_channel_id,
            user_channel_id: connection_result.user_channel_id,
            no_server_pointer: connection_result.no_server_pointer,
        }
        .build();

        Self {
            x224_processor,
            fast_path_processor,
            no_server_pointer: connection_result.no_server_pointer,
        }
    }

    pub fn update_mouse_pos(&mut self, x: usize, y: usize) {
        self.fast_path_processor.update_mouse_pos(x, y);
    }

    /// Encodes outgoing input events and modifies image if necessary (e.g for client-side pointer
    /// rendering).
    pub fn process_fastpath_input(
        &mut self,
        image: &mut DecodedImage,
        events: &[FastPathInputEvent],
    ) -> SessionResult<Vec<ActiveStageOutput>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }

        // Mouse move events are prevalent, so we can preallocate space for
        // response frame + graphics update
        let mut output = Vec::with_capacity(2);

        // Encoding fastpath response frame
        // PERF: unnecessary copy
        let fastpath_input = FastPathInput(events.to_vec());
        let mut frame = Vec::new();
        fastpath_input
            .to_buffer(&mut frame)
            .map_err(|e| custom_err!("FastPathInput encode", e))?;
        output.push(ActiveStageOutput::ResponseFrame(frame));

        // If pointer rendering is disabled - we can skip the rest
        if self.no_server_pointer {
            return Ok(output);
        }

        // If mouse was moved by client - we should update framebuffer to reflect new
        // pointer position
        let mouse_pos = events.iter().find_map(|event| match event {
            FastPathInputEvent::MouseEvent(event) => Some((event.x_position as usize, event.y_position as usize)),
            FastPathInputEvent::MouseEventEx(event) => Some((event.x_position as usize, event.y_position as usize)),
            _ => None,
        });

        let (mouse_x, mouse_y) = match mouse_pos {
            Some(mouse_pos) => mouse_pos,
            None => return Ok(output),
        };

        // Graphics update is only sent when update is visually changed the framebuffer
        if let Some(rect) = image.move_pointer(mouse_x as u16, mouse_y as u16)? {
            output.push(ActiveStageOutput::GraphicsUpdate(rect));
        }

        Ok(output)
    }

    /// Process a frame received from the client.
    pub fn process(
        &mut self,
        image: &mut DecodedImage,
        action: Action,
        frame: &[u8],
    ) -> SessionResult<Vec<ActiveStageOutput>> {
        let (output, processor_updates) = match action {
            Action::FastPath => {
                let mut output = WriteBuf::new();
                let processor_updates = self.fast_path_processor.process(image, frame, &mut output)?;
                (output.into_inner(), processor_updates)
            }
            Action::X224 => (self.x224_processor.process(frame)?, Vec::new()),
        };

        let mut stage_outputs = Vec::new();

        if !output.is_empty() {
            stage_outputs.push(ActiveStageOutput::ResponseFrame(output));
        }

        for update in processor_updates {
            match update {
                UpdateKind::None => {}
                UpdateKind::Region(region) => {
                    stage_outputs.push(ActiveStageOutput::GraphicsUpdate(region));
                }
                UpdateKind::PointerDefault => {
                    stage_outputs.push(ActiveStageOutput::PointerDefault);
                }
                UpdateKind::PointerHidden => {
                    stage_outputs.push(ActiveStageOutput::PointerHidden);
                }
                UpdateKind::PointerPosition { x, y } => {
                    stage_outputs.push(ActiveStageOutput::PointerPosition { x, y });
                }
            }
        }

        Ok(stage_outputs)
    }

    /// Sends a PDU on the dynamic channel.
    pub fn encode_dynamic(&self, output: &mut WriteBuf, channel_name: &str, dvc_data: &[u8]) -> SessionResult<()> {
        self.x224_processor.encode_dynamic(output, channel_name, dvc_data)
    }

    /// Send a pdu on the static global channel. Typically used to send input events
    pub fn encode_static(
        &self,
        output: &mut WriteBuf,
        pdu: ironrdp_pdu::rdp::headers::ShareDataPdu,
    ) -> SessionResult<usize> {
        self.x224_processor.encode_static(output, pdu)
    }
}

#[derive(Debug)]
pub enum ActiveStageOutput {
    ResponseFrame(Vec<u8>),
    GraphicsUpdate(InclusiveRectangle),
    Terminate,
    PointerDefault,
    PointerHidden,
    PointerPosition { x: usize, y: usize },
}
