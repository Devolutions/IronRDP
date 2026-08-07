use std::sync::Arc;

use ironrdp_bulk::{BulkCompressor, CompressionType as BulkCompressionType};
use ironrdp_core::{ReadCursor, WriteBuf};
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_dvc::{DrdynvcClient, DvcClientProcessor, DynamicChannelRef};
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::rdp::autodetect::AutoDetectRequest;
use ironrdp_pdu::rdp::client_info::CompressionType;
use ironrdp_pdu::rdp::headers::ShareDataPdu;
use ironrdp_pdu::rdp::multitransport::MultitransportRequestPdu;
use ironrdp_pdu::rdp::refresh_rectangle::RefreshRectanglePdu;
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
use ironrdp_pdu::rdp::suppress_output::SuppressOutputPdu;
use ironrdp_pdu::slow_path::{self, GraphicsUpdateType};
use ironrdp_pdu::{Action, mcs};
use ironrdp_svc::{StaticChannelSet, SvcMessage, SvcProcessor, SvcProcessorMessages};
use tracing::{debug, warn};

use crate::fast_path::UpdateKind;
use crate::image::DecodedImage;
use crate::{SessionError, SessionErrorExt as _, SessionResult, fast_path, x224};

fn to_bulk_compression_type(compression_type: CompressionType) -> BulkCompressionType {
    match compression_type {
        CompressionType::K8 => BulkCompressionType::Rdp4,
        CompressionType::K64 => BulkCompressionType::Rdp5,
        CompressionType::Rdp6 => BulkCompressionType::Rdp6,
        CompressionType::Rdp61 => BulkCompressionType::Rdp61,
    }
}

pub struct ActiveStage {
    x224_processor: x224::Processor,
    fast_path_processor: fast_path::Processor,
    /// Shared server-to-client compression history across all output transports.
    bulk_decompressor: Option<BulkCompressor>,
    enable_server_pointer: bool,
}

/// Builder for [`ActiveStage`].
///
/// All fields are required; they are typically taken straight from `ironrdp-connector`’s
/// `ConnectionResult` once the connection sequence is finalized.
pub struct ActiveStageBuilder {
    pub static_channels: StaticChannelSet,
    pub user_channel_id: u16,
    pub io_channel_id: u16,
    pub message_channel_id: Option<u16>,
    pub share_id: u32,
    /// The bulk compression type negotiated during connection activation.
    pub compression_type: Option<CompressionType>,
    /// Enable server-side pointer updates (client-side pointer rendering).
    pub enable_server_pointer: bool,
    /// Use software rendering mode for pointer bitmap generation.
    pub pointer_software_rendering: bool,
}

impl ActiveStageBuilder {
    pub fn build(self) -> ActiveStage {
        let Self {
            static_channels,
            user_channel_id,
            io_channel_id,
            message_channel_id,
            share_id,
            compression_type,
            enable_server_pointer,
            pointer_software_rendering,
        } = self;

        let x224_processor = x224::Processor::new(
            static_channels,
            user_channel_id,
            io_channel_id,
            message_channel_id,
            share_id,
        );

        let fast_path_processor = fast_path::ProcessorBuilder {
            io_channel_id,
            user_channel_id,
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
        }
        .build();

        ActiveStage {
            x224_processor,
            fast_path_processor,
            bulk_decompressor: new_bulk_decompressor(compression_type),
            enable_server_pointer,
        }
    }
}

fn new_bulk_decompressor(compression_type: Option<CompressionType>) -> Option<BulkCompressor> {
    compression_type.map(|compression_type| BulkCompressor::new(to_bulk_compression_type(compression_type)))
}

impl ActiveStage {
    pub fn update_mouse_pos(&mut self, x: u16, y: u16) {
        self.fast_path_processor.update_mouse_pos(x, y);
    }

    #[must_use]
    pub fn set_static_channel_chunk_size(&mut self, chunk_size: usize) -> bool {
        self.x224_processor.set_static_channel_chunk_size(chunk_size)
    }

    /// Returns whether a malformed Fast-Path bitmap was discarded and needs a full visual recovery.
    ///
    /// The caller decides whether the negotiated capabilities permit a recovery request.
    pub fn take_bitmap_recovery_request(&mut self) -> bool {
        self.fast_path_processor.take_bitmap_recovery_request()
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

        // The Fast-Path event count is an 8-bit field, so preserve input order across bounded frames.
        for event_chunk in events.chunks(FastPathInput::MAX_EVENTS) {
            // PERF: unnecessary copy
            let fastpath_input = FastPathInput::new(event_chunk.to_vec()).map_err(SessionError::decode)?;
            let frame = ironrdp_core::encode_vec(&fastpath_input).map_err(SessionError::encode)?;
            output.push(ActiveStageOutput::ResponseFrame(frame));
        }

        // If pointer rendering is disabled - we can skip the rest
        if !self.enable_server_pointer {
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
                let processor_updates =
                    self.fast_path_processor
                        .process(image, frame, &mut output, &mut self.bulk_decompressor)?;
                (
                    vec![ActiveStageOutput::ResponseFrame(output.into_inner())],
                    processor_updates,
                )
            }
            Action::X224 => {
                let x224_outputs = self.x224_processor.process(frame, &mut self.bulk_decompressor)?;
                let mut stage_outputs = Vec::new();
                let mut processor_updates = Vec::new();

                for output in x224_outputs {
                    match output {
                        x224::ProcessorOutput::GraphicsUpdate(data) => {
                            let updates = process_slow_path_graphics(&mut self.fast_path_processor, image, &data)?;
                            processor_updates.extend(updates);
                        }
                        x224::ProcessorOutput::PointerUpdate(data) => {
                            let updates = process_slow_path_pointer(&mut self.fast_path_processor, image, &data)?;
                            processor_updates.extend(updates);
                        }
                        other => {
                            stage_outputs.push(ActiveStageOutput::try_from(other)?);
                        }
                    }
                }

                (stage_outputs, processor_updates)
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

    /// Replaces the fast-path processor wholesale.
    ///
    /// Prefer [`ActiveStage::reactivate`] for a Deactivation-Reactivation Sequence: it also
    /// updates the share_id and the server-pointer setting, which a bare replacement does not.
    pub fn set_fastpath_processor(&mut self, processor: fast_path::Processor) {
        self.fast_path_processor = processor;
    }

    /// Updates the share_id used by the x224 processor for encoding ShareDataPdu.
    ///
    /// [`ActiveStage::reactivate`] already does this, so a Deactivation-Reactivation Sequence does
    /// not need to call it.
    pub fn set_share_id(&mut self, share_id: u32) {
        self.x224_processor.set_share_id(share_id);
    }

    pub fn set_enable_server_pointer(&mut self, enable_server_pointer: bool) {
        self.enable_server_pointer = enable_server_pointer;
    }

    /// Rebuilds the fast-path processor for a [Deactivation-Reactivation Sequence].
    ///
    /// The shared bulk decompression history is retained. The server signals any history reset
    /// with the PACKET_FLUSHED and PACKET_AT_FRONT compression flags, which are applied per update.
    ///
    /// [Deactivation-Reactivation Sequence]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
    pub fn reactivate(
        &mut self,
        io_channel_id: u16,
        user_channel_id: u16,
        share_id: u32,
        enable_server_pointer: bool,
        pointer_software_rendering: bool,
    ) {
        self.fast_path_processor = fast_path::ProcessorBuilder {
            io_channel_id,
            user_channel_id,
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
        }
        .build();
        // The x224 processor encodes ShareDataPdu with the server's (possibly new) share_id.
        self.x224_processor.set_share_id(share_id);
        self.enable_server_pointer = enable_server_pointer;
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

    /// Requests a full redraw of the negotiated desktop.
    fn request_full_refresh(&self, width: u16, height: u16) -> SessionResult<Vec<u8>> {
        debug_assert!(width != 0 && height != 0);
        let mut frame = WriteBuf::new();
        self.x224_processor.encode_static(
            &mut frame,
            ShareDataPdu::RefreshRectangle(RefreshRectanglePdu {
                areas_to_refresh: vec![InclusiveRectangle {
                    left: 0,
                    top: 0,
                    right: width.saturating_sub(1),
                    bottom: height.saturating_sub(1),
                }],
            }),
        )?;
        Ok(frame.into_inner())
    }

    /// Requests a full redraw using a server-supported recovery PDU.
    ///
    /// A Suppress Output toggle is preferred when supported because it is the documented Refresh
    /// Rect workaround for affected Microsoft RDP servers.
    ///
    /// [MS-RDPBCGR 2.2.11.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/0be71491-0b01-402c-947d-080706ccf91b
    pub fn request_full_redraw(
        &self,
        width: u16,
        height: u16,
        refresh_rect_support: bool,
        suppress_output_support: bool,
    ) -> SessionResult<Vec<Vec<u8>>> {
        debug_assert!(width != 0 && height != 0);
        if suppress_output_support {
            let mut suppress = WriteBuf::new();
            self.x224_processor.encode_static(
                &mut suppress,
                ShareDataPdu::SuppressOutput(SuppressOutputPdu { desktop_rect: None }),
            )?;

            let mut resume = WriteBuf::new();
            self.x224_processor.encode_static(
                &mut resume,
                ShareDataPdu::SuppressOutput(SuppressOutputPdu {
                    desktop_rect: Some(InclusiveRectangle {
                        left: 0,
                        top: 0,
                        right: width.saturating_sub(1),
                        bottom: height.saturating_sub(1),
                    }),
                }),
            )?;

            return Ok(vec![suppress.into_inner(), resume.into_inner()]);
        }

        if refresh_rect_support {
            return Ok(vec![self.request_full_refresh(width, height)?]);
        }

        Ok(Vec::new())
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

    pub fn get_dvc<T: DvcClientProcessor + 'static>(&self) -> Option<DynamicChannelRef<'_, T>> {
        self.x224_processor.get_dvc::<T>()
    }

    pub fn get_dvc_by_channel_id<T: DvcClientProcessor + 'static>(
        &self,
        channel_id: u32,
    ) -> Option<DynamicChannelRef<'_, T>> {
        self.x224_processor.get_dvc_by_channel_id(channel_id)
    }

    /// Returns whether the Display Control channel is available and has received server capabilities.
    ///
    /// `None` means no Display Control client is configured. `Some(false)` means it is configured
    /// but its dynamic channel is still opening or has not received its capabilities PDU.
    pub fn display_control_ready(&mut self) -> Option<bool> {
        let Some(dvc) = self.get_dvc::<DisplayControlClient>() else {
            return self
                .get_svc_processor::<DrdynvcClient>()
                .and_then(|drdynvc| drdynvc.has_registered_dvc::<DisplayControlClient>().then_some(false));
        };
        Some(dvc.processor().ready())
    }

    /// Completes user's SVC request with data, required to sent it over the network and returns
    /// a buffer with encoded data.
    pub fn process_svc_processor_messages<C: SvcProcessor + 'static>(
        &self,
        messages: SvcProcessorMessages<C>,
    ) -> SessionResult<Vec<u8>> {
        self.x224_processor.process_svc_processor_messages(messages)
    }

    /// Completes an SVC request for a runtime-defined channel name.
    pub fn process_svc_messages_by_name(
        &self,
        channel_name: &ChannelName,
        messages: Vec<SvcMessage>,
    ) -> SessionResult<Vec<u8>> {
        self.x224_processor.process_svc_messages_by_name(channel_name, messages)
    }

    /// Fully encodes a resize request for sending over the Display Control Virtual Channel.
    ///
    /// If the Display Control Virtual Channel is not available, not yet connected, or has not
    /// received its required server capabilities PDU, this method returns `None`.
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
            let channel_id = dvc.channel_id();
            let display_control = dvc.processor();
            if !display_control.ready() {
                debug!("Could not encode a resize: Display Control capabilities have not been received");
                return None;
            }
            let svc_messages = match display_control.encode_single_primary_monitor(
                channel_id,
                width,
                height,
                scale_factor,
                physical_dims,
            ) {
                Ok(messages) => messages,
                Err(e) => return Some(Err(SessionError::encode(e))),
            };

            return Some(self.process_svc_processor_messages(SvcProcessorMessages::<DrdynvcClient>::new(svc_messages)));
        } else {
            debug!("Could not encode a resize: Display Control Virtual Channel is not available");
        }

        None
    }

    pub fn encode_dvc_messages(&mut self, messages: Vec<SvcMessage>) -> SessionResult<Vec<u8>> {
        self.process_svc_processor_messages(SvcProcessorMessages::<DrdynvcClient>::new(messages))
    }
}

#[derive(Debug)]
pub enum ActiveStageOutput {
    ResponseFrame(Vec<u8>),
    GraphicsUpdate(InclusiveRectangle),
    PointerDefault,
    PointerHidden,
    PointerPosition {
        x: u16,
        y: u16,
    },
    PointerBitmap(Arc<DecodedPointer>),
    Terminate(GracefulDisconnectReason),
    /// Server Save Session Info notification ([MS-RDPBCGR] 2.2.10.1).
    ///
    /// This value-free event deliberately excludes server-provided session details, which can
    /// include credentials and auto-reconnect cookies.
    SaveSessionInfo {
        /// Whether the notification unambiguously reports a completed logon.
        logon_complete: bool,
    },
    /// Received a Server Deactivate All PDU. The consumer should execute the [Deactivation-Reactivation Sequence].
    ///
    /// [Deactivation-Reactivation Sequence]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
    DeactivateAll,
    /// Server Initiate Multitransport Request. The application should establish a
    /// sideband UDP transport using the provided request parameters.
    ///
    /// See [\[MS-RDPBCGR\] 2.2.15.1].
    ///
    /// [\[MS-RDPBCGR\] 2.2.15.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/de783158-8b01-4818-8fb0-62523a5b3490
    MultitransportRequest(MultitransportRequestPdu),
    /// Server-reported network characteristics ([\[MS-RDPBCGR\] 2.2.14.1.5]).
    ///
    /// Contains an [`AutoDetectRequest::NetworkCharacteristicsResult`] with
    /// RTT and/or bandwidth measurements computed by the server.
    ///
    /// See [\[MS-RDPBCGR\] 2.2.14.1.5].
    ///
    /// [\[MS-RDPBCGR\] 2.2.14.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/228ffc5c-b60c-4d3e-9781-ac613f822fdf
    AutoDetect(AutoDetectRequest),
    /// Server Auto-Reconnect Cookie ([\[MS-RDPBCGR\] 2.2.4.2]), received in a Save
    /// Session Info PDU.
    ///
    /// Hold this and pass it to `ClientConnector::with_auto_reconnect_cookie` when
    /// reconnecting after an ungraceful disconnect, so the server can reattach the
    /// session without a fresh logon. It can arrive more than once per session,
    /// since the server regenerates it hourly; keep the most recent.
    ///
    /// [\[MS-RDPBCGR\] 2.2.4.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/18f4f605-0ee3-4175-8a62-cf8775252547
    AutoReconnectCookie(ServerAutoReconnect),
}

impl TryFrom<x224::ProcessorOutput> for ActiveStageOutput {
    type Error = SessionError;

    fn try_from(value: x224::ProcessorOutput) -> Result<Self, Self::Error> {
        match value {
            x224::ProcessorOutput::ResponseFrame(frame) => Ok(Self::ResponseFrame(frame)),
            x224::ProcessorOutput::Disconnect(desc) => {
                let desc = match desc {
                    x224::DisconnectDescription::McsDisconnect(reason) => match reason {
                        mcs::DisconnectReason::ProviderInitiated => GracefulDisconnectReason::ServerInitiated,
                        mcs::DisconnectReason::UserRequested => GracefulDisconnectReason::UserInitiated,
                        other => GracefulDisconnectReason::Other(other.description().to_owned()),
                    },
                    x224::DisconnectDescription::ErrorInfo(info) => GracefulDisconnectReason::Other(info.description()),
                };

                Ok(Self::Terminate(desc))
            }
            x224::ProcessorOutput::SaveSessionInfo { logon_complete } => Ok(Self::SaveSessionInfo { logon_complete }),
            x224::ProcessorOutput::DeactivateAll => Ok(Self::DeactivateAll),
            x224::ProcessorOutput::MultitransportRequest(pdu) => Ok(Self::MultitransportRequest(pdu)),
            x224::ProcessorOutput::AutoDetect(request) => Ok(Self::AutoDetect(request)),
            x224::ProcessorOutput::AutoReconnectCookie(cookie) => Ok(Self::AutoReconnectCookie(cookie)),
            // GraphicsUpdate and PointerUpdate are consumed in ActiveStage::process()
            // before reaching this conversion.
            x224::ProcessorOutput::GraphicsUpdate(_) | x224::ProcessorOutput::PointerUpdate(_) => Err(
                SessionError::general("slow-path graphics/pointer updates should be handled before this conversion"),
            ),
        }
    }
}

/// Reasons for graceful disconnect. This type provides GUI-friendly descriptions for
/// disconnect reasons.
#[derive(Debug, Clone)]
pub enum GracefulDisconnectReason {
    UserInitiated,
    ServerInitiated,
    Other(String),
}

impl GracefulDisconnectReason {
    pub fn description(&self) -> String {
        match self {
            GracefulDisconnectReason::UserInitiated => "user initiated disconnect".to_owned(),
            GracefulDisconnectReason::ServerInitiated => "server initiated disconnect".to_owned(),
            GracefulDisconnectReason::Other(description) => description.clone(),
        }
    }
}

impl core::fmt::Display for GracefulDisconnectReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.description())
    }
}

/// Parse and process a slow-path graphics update through the shared bitmap pipeline.
fn process_slow_path_graphics(
    fast_path_processor: &mut fast_path::Processor,
    image: &mut DecodedImage,
    data: &[u8],
) -> SessionResult<Vec<UpdateKind>> {
    let mut src = ReadCursor::new(data);
    let update_type = slow_path::read_graphics_update_type(&mut src).map_err(SessionError::decode)?;

    match update_type {
        GraphicsUpdateType::Bitmap => {
            let bitmap = slow_path::decode_slow_path_bitmap(&mut src).map_err(SessionError::decode)?;
            fast_path_processor.process_bitmap_update(image, bitmap)
        }
        GraphicsUpdateType::Orders => {
            warn!("Slow-path drawing orders not supported (MS-RDPEGDI)");
            Ok(Vec::new())
        }
        GraphicsUpdateType::Palette => {
            fast_path_processor.process_palette_update(data);
            Ok(Vec::new())
        }
        // Synchronize is an artifact from the T.128 multipoint protocol
        // and carries no data. Safe to ignore.
        GraphicsUpdateType::Synchronize => {
            debug!("Ignoring slow-path synchronize update");
            Ok(Vec::new())
        }
    }
}

/// Parse and process a slow-path pointer update through the shared pointer pipeline.
fn process_slow_path_pointer(
    fast_path_processor: &mut fast_path::Processor,
    image: &mut DecodedImage,
    data: &[u8],
) -> SessionResult<Vec<UpdateKind>> {
    let mut src = ReadCursor::new(data);
    let pointer = slow_path::decode_slow_path_pointer(&mut src).map_err(SessionError::decode)?;
    fast_path_processor.process_pointer_update(image, pointer)
}

#[cfg(test)]
mod tests {
    use ironrdp_core::decode;
    use ironrdp_graphics::image_processing::PixelFormat;
    use ironrdp_pdu::input::fast_path::KeyboardFlags;
    use ironrdp_pdu::pointer::{ColorPointerAttribute, Point16, PointerAttribute, PointerUpdateData};

    use super::*;

    #[test]
    fn full_redraw_prefers_suppress_output_toggle_when_supported() {
        let stage = ActiveStageBuilder {
            static_channels: StaticChannelSet::new(),
            user_channel_id: 1001,
            io_channel_id: 1003,
            message_channel_id: None,
            share_id: 1,
            compression_type: None,
            enable_server_pointer: true,
            pointer_software_rendering: false,
        }
        .build();

        let suppress_output_frames = stage.request_full_redraw(1024, 768, true, true).unwrap();
        assert_eq!(suppress_output_frames.len(), 2);
        assert!(suppress_output_frames.iter().all(|frame| !frame.is_empty()));

        assert_eq!(stage.request_full_redraw(1024, 768, true, false).unwrap().len(), 1);
        assert!(stage.request_full_redraw(1024, 768, false, false).unwrap().is_empty());
    }

    #[test]
    fn fastpath_input_splits_event_counts_that_exceed_the_wire_limit() {
        let mut stage = ActiveStageBuilder {
            static_channels: StaticChannelSet::new(),
            user_channel_id: 1001,
            io_channel_id: 1003,
            message_channel_id: None,
            share_id: 1,
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 1, 1);
        let events = vec![
            FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), u16::from(b'a'));
            FastPathInput::MAX_EVENTS + 1
        ];

        let outputs = stage
            .process_fastpath_input(&mut image, &events)
            .expect("large input event batch should encode");
        let event_counts = outputs
            .iter()
            .filter_map(|output| match output {
                ActiveStageOutput::ResponseFrame(frame) => Some(
                    decode::<FastPathInput>(frame)
                        .expect("response frame should decode")
                        .input_events()
                        .len(),
                ),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(event_counts, [FastPathInput::MAX_EVENTS, 1]);
    }

    #[test]
    fn slow_path_palette_applies_to_indexed_pointer() {
        let mut palette_data = vec![0; 8 + 256 * 3];
        palette_data[0..2].copy_from_slice(&0x0002u16.to_le_bytes());
        palette_data[4..8].copy_from_slice(&256u32.to_le_bytes());
        palette_data[8 + 3..8 + 6].copy_from_slice(&[0x10, 0x20, 0x30]);

        let mut processor = fast_path::ProcessorBuilder {
            io_channel_id: 0,
            user_channel_id: 0,
            share_id: 0,
            enable_server_pointer: true,
            pointer_software_rendering: false,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 1, 1);

        let palette_updates = process_slow_path_graphics(&mut processor, &mut image, &palette_data)
            .expect("slow-path palette update should succeed");
        assert!(palette_updates.is_empty());

        let pointer = PointerAttribute {
            xor_bpp: 8,
            color_pointer: ColorPointerAttribute {
                cache_index: 0,
                hot_spot: Point16 { x: 0, y: 0 },
                width: 1,
                height: 1,
                xor_mask: &[1, 0],
                and_mask: &[0, 0],
            },
        };
        let pointer_updates = processor
            .process_pointer_update(&mut image, PointerUpdateData::New(pointer))
            .expect("indexed pointer should decode with slow-path palette");

        let [UpdateKind::PointerBitmap(pointer)] = pointer_updates.as_slice() else {
            panic!("expected an accelerated pointer bitmap");
        };
        assert_eq!(pointer.bitmap_data, [0x10, 0x20, 0x30, 0xff]);
    }
}
