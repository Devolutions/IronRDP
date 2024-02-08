use std::rc::Rc;

use ironrdp_connector::ConnectionResult;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::rdp::headers::ShareDataPdu;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{mcs, Action};
use ironrdp_svc::{SvcProcessor, SvcProcessorMessages};

use crate::fast_path::UpdateKind;
use crate::image::DecodedImage;
use crate::x224::GfxHandler;
use crate::{fast_path, x224, SessionError, SessionErrorExt, SessionResult};

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
            pointer_software_rendering: connection_result.pointer_software_rendering,
        }
        .build();

        Self {
            x224_processor,
            fast_path_processor,
            no_server_pointer: connection_result.no_server_pointer,
        }
    }

    pub fn update_mouse_pos(&mut self, x: u16, y: u16) {
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
        let frame = ironrdp_pdu::encode_vec(&fastpath_input).map_err(SessionError::pdu)?;
        output.push(ActiveStageOutput::ResponseFrame(frame));

        // If pointer rendering is disabled - we can skip the rest
        if self.no_server_pointer {
            return Ok(output);
        }

        // If mouse was moved by client - we should update framebuffer to reflect new
        // pointer position
        let mouse_pos = events.iter().find_map(|event| match event {
            FastPathInputEvent::MouseEvent(event) => Some((event.x_position, event.y_position)),
            FastPathInputEvent::MouseEventEx(event) => Some((event.x_position, event.y_position)),
            _ => None,
        });

        let (mouse_x, mouse_y) = match mouse_pos {
            Some(mouse_pos) => mouse_pos,
            None => return Ok(output),
        };

        // Graphics update is only sent when update is visually changed the framebuffer
        if let Some(rect) = image.move_pointer(mouse_x, mouse_y)? {
            output.push(ActiveStageOutput::GraphicsUpdate(rect));
        }

        Ok(output)
    }

    /// Process a frame received from the server.
    pub fn process(
        &mut self,
        image: &mut DecodedImage,
        action: Action,
        frame: &[u8],
    ) -> SessionResult<Vec<ActiveStageOutput>> {
        let (mut stage_outputs, processor_updates) = match action {
            Action::FastPath => {
                let mut output = WriteBuf::new();
                let processor_updates = self.fast_path_processor.process(image, frame, &mut output)?;
                (
                    vec![ActiveStageOutput::ResponseFrame(output.into_inner())],
                    processor_updates,
                )
            }
            Action::X224 => {
                let outputs = self
                    .x224_processor
                    .process(frame)?
                    .into_iter()
                    .map(TryFrom::try_from)
                    .collect::<Result<Vec<_>, _>>()?;
                (outputs, Vec::new())
            }
        };

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
                UpdateKind::PointerBitmap(pointer) => {
                    stage_outputs.push(ActiveStageOutput::PointerBitmap(pointer));
                }
            }
        }

        Ok(stage_outputs)
    }

    /// Encodes client-side graceful shutdown request. Note that upon sending this request,
    /// client should wait for server's ShutdownDenied PDU before closing the connection.
    ///
    /// Client-side graceful shutdown is defined in [MS-RDPBCGR]
    ///
    /// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/27915739-8f77-487e-9927-55008af7fd68
    pub fn graceful_shutdown(&self) -> SessionResult<Vec<ActiveStageOutput>> {
        let mut frame = WriteBuf::new();
        self.x224_processor
            .encode_static(&mut frame, ShareDataPdu::ShutdownRequest)?;

        Ok(vec![ActiveStageOutput::ResponseFrame(frame.into_inner())])
    }

    /// Sends a PDU on the dynamic channel.
    pub fn encode_dynamic(&self, output: &mut WriteBuf, channel_name: &str, dvc_data: &[u8]) -> SessionResult<()> {
        self.x224_processor.encode_dynamic(output, channel_name, dvc_data)
    }

    /// Send a pdu on the static global channel. Typically used to send input events
    pub fn encode_static(&self, output: &mut WriteBuf, pdu: ShareDataPdu) -> SessionResult<usize> {
        self.x224_processor.encode_static(output, pdu)
    }

    pub fn get_svc_processor<T: SvcProcessor + 'static>(&mut self) -> Option<&T> {
        self.x224_processor.get_svc_processor()
    }

    pub fn get_svc_processor_mut<T: SvcProcessor + 'static>(&mut self) -> Option<&mut T> {
        self.x224_processor.get_svc_processor_mut()
    }

    /// Completes user's SVC request with data, required to sent it over the network and returns
    /// a buffer with encoded data.
    pub fn process_svc_processor_messages<C: SvcProcessor + 'static>(
        &self,
        messages: SvcProcessorMessages<C>,
    ) -> SessionResult<Vec<u8>> {
        self.x224_processor.process_svc_processor_messages(messages)
    }
}

#[derive(Debug)]
pub enum ActiveStageOutput {
    ResponseFrame(Vec<u8>),
    GraphicsUpdate(InclusiveRectangle),
    PointerDefault,
    PointerHidden,
    PointerPosition { x: u16, y: u16 },
    PointerBitmap(Rc<DecodedPointer>),
    Terminate(GracefulDisconnectReason),
}

impl TryFrom<x224::ProcessorOutput> for ActiveStageOutput {
    type Error = SessionError;

    fn try_from(value: x224::ProcessorOutput) -> Result<Self, Self::Error> {
        match value {
            x224::ProcessorOutput::ResponseFrame(frame) => Ok(Self::ResponseFrame(frame)),
            x224::ProcessorOutput::Disconnect(reason) => {
                let reason = match reason {
                    mcs::DisconnectReason::UserRequested => GracefulDisconnectReason::UserInitiated,
                    mcs::DisconnectReason::ProviderInitiated => GracefulDisconnectReason::ServerInitiated,
                    other => GracefulDisconnectReason::Other(other.description()),
                };

                Ok(Self::Terminate(reason))
            }
        }
    }
}

/// Reasons for graceful disconnect. This type provides GUI-friendly descriptions for
/// disconnect reasons.
#[derive(Debug, Clone, Copy)]
pub enum GracefulDisconnectReason {
    UserInitiated,
    ServerInitiated,
    Other(&'static str),
}

impl GracefulDisconnectReason {
    pub fn description(&self) -> &'static str {
        match self {
            GracefulDisconnectReason::UserInitiated => "user initiated disconnect",
            GracefulDisconnectReason::ServerInitiated => "server initiated disconnect",
            GracefulDisconnectReason::Other(description) => description,
        }
    }
}

impl core::fmt::Display for GracefulDisconnectReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.description())
    }
}
