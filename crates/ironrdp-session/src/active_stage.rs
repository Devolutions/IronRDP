use std::rc::Rc;

use ironrdp_connector::connection_activation::ConnectionActivationSequence;
use ironrdp_connector::ConnectionResult;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_dvc::{DrdynvcClient, DvcProcessor, DynamicVirtualChannel};
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::rdp::headers::ShareDataPdu;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{mcs, Action};
use ironrdp_svc::{SvcProcessor, SvcProcessorMessages};

use crate::fast_path::UpdateKind;
use crate::image::DecodedImage;
use crate::{fast_path, x224, SessionError, SessionErrorExt, SessionResult};

pub struct ActiveStage {
    x224_processor: x224::Processor,
    fast_path_processor: fast_path::Processor,
    no_server_pointer: bool,
}

impl ActiveStage {
    pub fn new(connection_result: ConnectionResult) -> Self {
        let x224_processor = x224::Processor::new(
            connection_result.static_channels,
            connection_result.user_channel_id,
            connection_result.io_channel_id,
            connection_result.connection_activation,
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

    pub fn set_fastpath_processor(&mut self, processor: fast_path::Processor) {
        self.fast_path_processor = processor;
    }

    pub fn set_no_server_pointer(&mut self, no_server_pointer: bool) {
        self.no_server_pointer = no_server_pointer;
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

    pub fn get_dvc<T: DvcProcessor + 'static>(&mut self) -> Option<&DynamicVirtualChannel> {
        self.x224_processor.get_dvc::<T>()
    }

    /// Completes user's SVC request with data, required to sent it over the network and returns
    /// a buffer with encoded data.
    pub fn process_svc_processor_messages<C: SvcProcessor + 'static>(
        &self,
        messages: SvcProcessorMessages<C>,
    ) -> SessionResult<Vec<u8>> {
        self.x224_processor.process_svc_processor_messages(messages)
    }

    /// Fully encodes a resize request for sending over the Display Control Virtual Channel.
    ///
    /// If the Display Control Virtual Channel is not available, or not yet connected, this method
    /// will return `None`.
    ///
    /// Per [2.2.2.2.1]:
    /// - The `width` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels, and MUST NOT be an odd value.
    /// - The `height` MUST be greater than or equal to 200 pixels and less than or equal to 8192 pixels.
    /// - The `scale_factor` MUST be ignored if it is less than 100 percent or greater than 500 percent.
    /// - The `physical_dims` (width, height) MUST be ignored if either is less than 10 mm or greater than 10,000 mm.
    ///
    /// Use [`ironrdp_displaycontrol::pdu::MonitorLayoutEntry::adjust_display_size`] to adjust `width` and `height` before calling this function
    /// to ensure the display size is within the valid range.
    ///
    /// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    pub fn encode_resize(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: Option<u32>,
        physical_dims: Option<(u32, u32)>,
    ) -> Option<SessionResult<Vec<u8>>> {
        if let Some(dvc) = self.get_dvc::<DisplayControlClient>() {
            if dvc.is_open() {
                let display_control = dvc.channel_processor_downcast_ref::<DisplayControlClient>()?;
                let channel_id = dvc.channel_id().unwrap(); // Safe to unwrap, as we checked if the channel is open
                let svc_messages = match display_control.encode_single_primary_monitor(
                    channel_id,
                    width,
                    height,
                    scale_factor,
                    physical_dims,
                ) {
                    Ok(messages) => messages,
                    Err(e) => return Some(Err(SessionError::pdu(e))),
                };

                return Some(
                    self.process_svc_processor_messages(SvcProcessorMessages::<DrdynvcClient>::new(svc_messages)),
                );
            } else {
                debug!("Could not encode a resize: Display Control Virtual Channel is not yet connected");
            }
        } else {
            debug!("Could not encode a resize: Display Control Virtual Channel is not available");
        }

        None
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
    DeactivateAll(Box<ConnectionActivationSequence>),
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
            x224::ProcessorOutput::DeactivateAll(cas) => Ok(Self::DeactivateAll(cas)),
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
