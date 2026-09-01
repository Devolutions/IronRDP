use std::sync::Arc;

use ironrdp_bulk::{BulkCompressor, CompressionType as BulkCompressionType};
use ironrdp_core::{ReadCursor, WriteBuf};
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_dvc::pdu::SoftSyncTunnelType;
use ironrdp_dvc::{DrdynvcClient, DvcClientProcessor, DvcMessageBatch, DynamicChannelMut, DynamicChannelRef};
use ironrdp_egfx::client::GraphicsPipelineClient;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::gcc::{ChannelName, Monitor};
use ironrdp_pdu::geometry::{ExclusiveRectangle, InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::rdp::autodetect::AutoDetectRequest;
use ironrdp_pdu::rdp::capability_sets::WindowSupportLevel;
use ironrdp_pdu::rdp::client_info::CompressionType;
use ironrdp_pdu::rdp::headers::ShareDataPdu;
use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, MultitransportResponsePdu};
use ironrdp_pdu::rdp::refresh_rectangle::RefreshRectanglePdu;
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
use ironrdp_pdu::rdp::suppress_output::SuppressOutputPdu;
use ironrdp_pdu::slow_path::{self, GraphicsUpdateType};
use ironrdp_pdu::window::{
    WindowingOrdersUpdate, try_decode_fast_path_windowing_orders, try_decode_slow_path_windowing_orders,
};
use ironrdp_pdu::{Action, mcs};
use ironrdp_rdpei::RdpeiClient;
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
    window_support_level: Option<WindowSupportLevel>,
    graphics_output_needs_full_refresh: bool,
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
            window_support_level: None,
            graphics_output_needs_full_refresh: false,
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
        // response frames + graphics update.
        let mut output = Vec::with_capacity(events.len().div_ceil(FastPathInput::MAX_EVENTS) + 1);

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
        let mouse_pos = events.iter().rev().find_map(|event| match event {
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
                            let (updates, windowing_orders) = process_slow_path_graphics(
                                &mut self.fast_path_processor,
                                image,
                                self.window_support_level,
                                &data,
                            )?;
                            processor_updates.extend(updates);
                            if let Some(windowing_orders) = windowing_orders {
                                stage_outputs.push(ActiveStageOutput::WindowingOrders(windowing_orders));
                            }
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

                // Drain the client-side EGFX compositor: composite each completed-frame
                // output region into the image and surface it as a graphics update. EGFX
                // data only ever arrives over a DVC, which is X224-carried, so this stays
                // out of the Action::FastPath arm rather than running on every fast-path
                // frame (the highest-frequency path in a session).
                let (output_reset, graphics_updates) = self
                    .get_dvc_mut::<GraphicsPipelineClient>()
                    .map(|mut gfx| {
                        let gfx = gfx.processor_mut();
                        (gfx.take_output_reset(), gfx.drain_output())
                    })
                    .unwrap_or_default();
                if let Some((width, height)) = output_reset {
                    *image = DecodedImage::new(PixelFormat::RgbA32, width, height);
                    self.graphics_output_needs_full_refresh = true;
                }
                if let Some(region) =
                    composite_graphics_updates(image, graphics_updates.into_iter().map(|u| (u.region, u.data)))?
                {
                    stage_outputs.push(ActiveStageOutput::GraphicsUpdate(region));
                }

                (stage_outputs, processor_updates)
            }
        };

        for update in processor_updates {
            match update {
                UpdateKind::None => {}
                UpdateKind::Orders(data) => {
                    if let Some(windowing_orders) =
                        process_fast_path_windowing_orders(self.window_support_level, &data)?
                    {
                        stage_outputs.push(ActiveStageOutput::WindowingOrders(windowing_orders));
                    }
                }
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

        if self.graphics_output_needs_full_refresh
            && let Some(ActiveStageOutput::GraphicsUpdate(region)) = stage_outputs
                .iter_mut()
                .find(|output| matches!(output, ActiveStageOutput::GraphicsUpdate(_)))
        {
            *region = InclusiveRectangle {
                left: 0,
                top: 0,
                right: image.width().saturating_sub(1),
                bottom: image.height().saturating_sub(1),
            };
            self.graphics_output_needs_full_refresh = false;
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

    /// Updates the negotiated maximum payload length of outgoing static virtual channel chunks.
    pub fn set_static_channel_chunk_size(&mut self, maximum_chunk_size: usize) -> bool {
        self.x224_processor.set_static_channel_chunk_size(maximum_chunk_size)
    }

    /// Returns the negotiated maximum payload length of outgoing static virtual channel chunks.
    pub fn static_channel_chunk_size(&self) -> usize {
        self.x224_processor.static_channel_chunk_size()
    }

    pub fn set_enable_server_pointer(&mut self, enable_server_pointer: bool) {
        self.enable_server_pointer = enable_server_pointer;
    }

    /// Sets Window List support for the current activation.
    ///
    /// `None` preserves desktop-session behavior by ignoring drawing orders.
    pub fn set_window_support_level(&mut self, window_support_level: Option<WindowSupportLevel>) {
        self.window_support_level = window_support_level;
    }

    /// Rebuilds the fast-path processor for a [Deactivation-Reactivation Sequence].
    ///
    /// The shared bulk decompression history is retained. The server signals any history reset
    /// with the PACKET_FLUSHED and PACKET_AT_FRONT compression flags, which are applied per update.
    ///
    /// Returns `false` without changing the active stage when the negotiated static virtual
    /// channel chunk size is invalid.
    ///
    /// [Deactivation-Reactivation Sequence]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
    pub fn reactivate(
        &mut self,
        io_channel_id: u16,
        user_channel_id: u16,
        share_id: u32,
        enable_server_pointer: bool,
        pointer_software_rendering: bool,
        static_channel_chunk_size: usize,
    ) -> bool {
        if !self
            .x224_processor
            .set_static_channel_chunk_size(static_channel_chunk_size)
        {
            return false;
        }

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

        true
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

    /// Encodes an Initiate Multitransport Response on the negotiated MCS message channel.
    pub fn encode_multitransport_response(&self, response: &MultitransportResponsePdu) -> SessionResult<Vec<u8>> {
        self.x224_processor.encode_multitransport_response(response)
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

    pub fn get_dvc_mut<T: DvcClientProcessor + 'static>(&mut self) -> Option<DynamicChannelMut<'_, T>> {
        self.x224_processor.get_dvc_mut::<T>()
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

    /// Marks the reliable UDP tunnel as available for DRDYNVC Soft-Sync.
    ///
    /// Call this after successfully sending the tunnel's Initiate Multitransport Response PDU.
    pub fn enable_reliable_udp_dvc_tunnel(&mut self) -> SessionResult<()> {
        self.get_svc_processor_mut::<DrdynvcClient>()
            .ok_or_else(|| SessionError::general("DRDYNVC static channel is not available"))?
            .enable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);
        Ok(())
    }

    /// Marks the reliable UDP tunnel as unavailable for future Soft-Sync requests.
    ///
    /// DVCs already routed through the tunnel are unaffected.
    pub fn disable_reliable_udp_dvc_tunnel(&mut self) -> SessionResult<()> {
        self.get_svc_processor_mut::<DrdynvcClient>()
            .ok_or_else(|| SessionError::general("DRDYNVC static channel is not available"))?
            .disable_soft_sync_tunnel(SoftSyncTunnelType::RELIABLE_UDP);
        Ok(())
    }

    /// Returns whether Soft-Sync moved any DVC to the reliable UDP tunnel.
    pub fn reliable_udp_dvc_tunnel_in_use(&self) -> bool {
        self.x224_processor
            .get_svc_processor::<DrdynvcClient>()
            .is_some_and(|drdynvc| drdynvc.has_channels_on_tunnel(SoftSyncTunnelType::RELIABLE_UDP))
    }

    /// Returns the Soft-Sync tunnel selected for client messages on `channel_id`.
    pub fn dvc_tunnel_for_channel(&self, channel_id: u32) -> Option<SoftSyncTunnelType> {
        self.x224_processor
            .get_svc_processor::<DrdynvcClient>()?
            .tunnel_for_channel(channel_id)
    }

    /// Processes an unframed DRDYNVC PDU received through a multitransport tunnel.
    ///
    /// Response messages remain unframed so the caller can encode them with
    /// [`SvcMessage::encode_unframed_pdu`] and send them through the selected tunnel.
    pub fn process_dvc_tunnel(
        &mut self,
        tunnel_type: SoftSyncTunnelType,
        payload: &[u8],
    ) -> SessionResult<DvcMessageBatch> {
        self.get_svc_processor_mut::<DrdynvcClient>()
            .ok_or_else(|| SessionError::general("DRDYNVC static channel is not available"))?
            .process_tunnel(tunnel_type, payload)
            .map_err(SessionError::pdu)
    }

    /// Prepares a resize request for routing over TCP or a Soft-Sync tunnel.
    ///
    /// If the Display Control Virtual Channel is not available, not yet connected, or has not
    /// received its required server capabilities PDU, this method returns `None`.
    pub fn prepare_resize(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: Option<u32>,
        physical_dims: Option<(u32, u32)>,
    ) -> Option<SessionResult<DvcMessageBatch>> {
        if let Some(dvc) = self.get_dvc::<DisplayControlClient>() {
            let channel_id = dvc.channel_id();
            let display_control = dvc.processor();
            if !display_control.ready() {
                debug!("Could not encode a resize: Display Control capabilities have not been received");
                return None;
            }
            let messages = match display_control.encode_single_primary_monitor(
                channel_id,
                width,
                height,
                scale_factor,
                physical_dims,
            ) {
                Ok(messages) => messages,
                Err(error) => return Some(Err(SessionError::encode(error))),
            };

            return Some(DvcMessageBatch::try_new(channel_id, messages).map_err(SessionError::pdu));
        }

        debug!("Could not encode a resize: Display Control Virtual Channel is not available");
        None
    }

    /// Fully encodes a resize request for sending over the Display Control Virtual Channel.
    ///
    /// If the Display Control Virtual Channel is not available, not yet connected, or has not
    /// received its required server capabilities PDU, this method returns `None`.
    /// Returns an error when Soft-Sync routes the channel through a multitransport tunnel.
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
    /// [2.2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
    pub fn encode_resize(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: Option<u32>,
        physical_dims: Option<(u32, u32)>,
    ) -> Option<SessionResult<Vec<u8>>> {
        let prepared = self.prepare_resize(width, height, scale_factor, physical_dims)?;
        Some(prepared.and_then(|batch| self.encode_dvc_batch_over_tcp(batch)))
    }

    /// Returns whether the RDPEI channel is available and ready (SC_READY / CS_READY exchanged).
    ///
    /// `None` means no RDPEI client is configured. `Some(false)` means it is registered but not
    /// yet ready (or currently suspended for touch/pen send purposes when checking readiness for
    /// injection — use [`Self::rdpei_can_send_touch`] for the send gate).
    pub fn rdpei_ready(&mut self) -> Option<bool> {
        let Some(dvc) = self.get_dvc::<RdpeiClient>() else {
            return self
                .get_svc_processor::<DrdynvcClient>()
                .and_then(|drdynvc| drdynvc.has_registered_dvc::<RdpeiClient>().then_some(false));
        };
        Some(dvc.processor().ready())
    }

    /// True when RDPEI is ready and the server has not suspended input.
    pub fn rdpei_can_send_touch(&mut self) -> bool {
        self.get_dvc::<RdpeiClient>()
            .is_some_and(|dvc| dvc.processor().ready() && !dvc.processor().is_suspended())
    }

    /// Prepares a touch event for routing over TCP or a Soft-Sync tunnel.
    ///
    /// Returns `None` when the channel is unavailable.
    /// Returns an error when the channel is not ready or suspended, or when the request cannot be encoded or batched.
    pub fn prepare_rdpei_touch(
        &mut self,
        event: ironrdp_rdpei::pdu::TouchEventPdu,
    ) -> Option<SessionResult<DvcMessageBatch>> {
        if let Some(dvc) = self.get_dvc::<RdpeiClient>() {
            let channel_id = dvc.channel_id();
            let rdpei = dvc.processor();
            if !rdpei.ready() {
                return Some(Err(SessionError::general("RDPEI channel is not ready")));
            }
            if rdpei.is_suspended() {
                return Some(Err(SessionError::general("RDPEI input is suspended")));
            }
            let messages = match rdpei.encode_touch_event(channel_id, event) {
                Ok(messages) => messages,
                Err(error) => return Some(Err(SessionError::encode(error))),
            };
            return Some(DvcMessageBatch::try_new(channel_id, messages).map_err(SessionError::pdu));
        } else {
            debug!("Could not encode RDPEI touch: Input Virtual Channel is not available");
        }
        None
    }

    /// Fully encodes a touch event for the RDPEI dynamic channel.
    ///
    /// Returns `None` when the channel is unavailable.
    /// Returns an error when preparation fails or Soft-Sync routes the channel through a multitransport tunnel.
    pub fn encode_rdpei_touch(&mut self, event: ironrdp_rdpei::pdu::TouchEventPdu) -> Option<SessionResult<Vec<u8>>> {
        let prepared = self.prepare_rdpei_touch(event)?;
        Some(prepared.and_then(|batch| self.encode_dvc_batch_over_tcp(batch)))
    }

    /// Prepares a dismiss-hovering-touch-contact PDU for routing over TCP or a Soft-Sync tunnel.
    pub fn prepare_rdpei_dismiss_hovering(&mut self, contact_id: u8) -> Option<SessionResult<DvcMessageBatch>> {
        if let Some(dvc) = self.get_dvc::<RdpeiClient>() {
            let channel_id = dvc.channel_id();
            let rdpei = dvc.processor();
            if !rdpei.ready() {
                debug!("Could not encode RDPEI dismiss hovering: channel is not ready");
                return None;
            }
            let messages = match rdpei.encode_dismiss_hovering(channel_id, contact_id) {
                Ok(messages) => messages,
                Err(error) => return Some(Err(SessionError::encode(error))),
            };
            return Some(DvcMessageBatch::try_new(channel_id, messages).map_err(SessionError::pdu));
        }
        None
    }

    /// Fully encodes a dismiss-hovering-touch-contact PDU on the RDPEI channel.
    ///
    /// Returns an error when Soft-Sync routes the channel through a multitransport tunnel.
    pub fn encode_rdpei_dismiss_hovering(&mut self, contact_id: u8) -> Option<SessionResult<Vec<u8>>> {
        let prepared = self.prepare_rdpei_dismiss_hovering(contact_id)?;
        Some(prepared.and_then(|batch| self.encode_dvc_batch_over_tcp(batch)))
    }

    /// Prepares a pen event for routing over TCP or a Soft-Sync tunnel.
    ///
    /// Returns `None` when the channel is unavailable, not ready, suspended, or pen is not allowed.
    pub fn prepare_rdpei_pen(
        &mut self,
        event: ironrdp_rdpei::pdu::PenEventPdu,
    ) -> Option<SessionResult<DvcMessageBatch>> {
        if let Some(dvc) = self.get_dvc::<RdpeiClient>() {
            let channel_id = dvc.channel_id();
            let rdpei = dvc.processor();
            if !rdpei.ready() {
                debug!("Could not encode RDPEI pen: channel is not ready");
                return None;
            }
            if rdpei.is_suspended() {
                debug!("Could not encode RDPEI pen: input is suspended");
                return None;
            }
            if !rdpei.pen_allowed() {
                debug!("Could not encode RDPEI pen: pen not allowed for negotiated version");
                return None;
            }
            let messages = match rdpei.encode_pen_event(channel_id, event) {
                Ok(messages) => messages,
                Err(error) => return Some(Err(SessionError::encode(error))),
            };
            return Some(DvcMessageBatch::try_new(channel_id, messages).map_err(SessionError::pdu));
        } else {
            debug!("Could not encode RDPEI pen: Input Virtual Channel is not available");
        }
        None
    }

    /// Fully encodes a pen event for the RDPEI dynamic channel.
    ///
    /// Returns `None` when the channel is unavailable, not ready, suspended, or pen is not allowed.
    /// Returns an error when Soft-Sync routes the channel through a multitransport tunnel.
    pub fn encode_rdpei_pen(&mut self, event: ironrdp_rdpei::pdu::PenEventPdu) -> Option<SessionResult<Vec<u8>>> {
        let prepared = self.prepare_rdpei_pen(event)?;
        Some(prepared.and_then(|batch| self.encode_dvc_batch_over_tcp(batch)))
    }

    pub fn encode_dvc_messages(&mut self, messages: Vec<SvcMessage>) -> SessionResult<Vec<u8>> {
        self.process_svc_processor_messages(SvcProcessorMessages::<DrdynvcClient>::new(messages))
    }

    fn encode_dvc_batch_over_tcp(&mut self, batch: DvcMessageBatch) -> SessionResult<Vec<u8>> {
        if self.dvc_tunnel_for_channel(batch.channel_id()).is_some() {
            return Err(SessionError::general(
                "dynamic channel is routed through a multitransport tunnel",
            ));
        }

        self.encode_dvc_messages(batch.into_messages())
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
    /// Server-reported remote monitor layout ([MS-RDPBCGR] 2.2.12.1).
    MonitorLayout(Vec<Monitor>),
    /// Validated Windowing Alternate Secondary Drawing Orders.
    ///
    /// The payload is a complete slow-path Orders graphics update. It remains
    /// protocol data rather than a RAIL message.
    WindowingOrders(Vec<u8>),
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
    /// Server rejected the automatic reconnection attempt.
    AutoReconnectFailed,
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
            x224::ProcessorOutput::AutoReconnectFailed => Ok(Self::AutoReconnectFailed),
            x224::ProcessorOutput::MonitorLayout(monitors) => Ok(Self::MonitorLayout(monitors)),
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
    window_support_level: Option<WindowSupportLevel>,
    data: &[u8],
) -> SessionResult<(Vec<UpdateKind>, Option<Vec<u8>>)> {
    let mut src = ReadCursor::new(data);
    let update_type = slow_path::read_graphics_update_type(&mut src).map_err(SessionError::decode)?;

    match update_type {
        GraphicsUpdateType::Bitmap => {
            let bitmap = slow_path::decode_slow_path_bitmap(&mut src).map_err(SessionError::decode)?;
            fast_path_processor
                .process_bitmap_update(image, bitmap)
                .map(|updates| (updates, None))
        }
        GraphicsUpdateType::Orders => {
            let Some(window_support_level) = window_support_level else {
                return Ok((Vec::new(), None));
            };
            let orders = try_decode_slow_path_windowing_orders(&mut src).map_err(SessionError::decode)?;
            validate_windowing_orders_support(&orders, window_support_level)?;
            Ok((Vec::new(), Some(data.to_vec())))
        }
        GraphicsUpdateType::Palette => {
            fast_path_processor.process_palette_update(data);
            Ok((Vec::new(), None))
        }
        // Synchronize is an artifact from the T.128 multipoint protocol
        // and carries no data. Safe to ignore.
        GraphicsUpdateType::Synchronize => {
            debug!("Ignoring slow-path synchronize update");
            Ok((Vec::new(), None))
        }
    }
}

fn process_fast_path_windowing_orders(
    window_support_level: Option<WindowSupportLevel>,
    data: &[u8],
) -> SessionResult<Option<Vec<u8>>> {
    let Some(window_support_level) = window_support_level else {
        return Ok(None);
    };

    let mut src = ReadCursor::new(data);
    let orders = try_decode_fast_path_windowing_orders(&mut src).map_err(SessionError::decode)?;
    validate_windowing_orders_support(&orders, window_support_level)?;

    let mut normalized = Vec::with_capacity(
        2 /* updateType */ + 2 /* pad2OctetsA */ + data.len() + 2, /* pad2OctetsB */
    );
    normalized.extend_from_slice(&0u16.to_le_bytes());
    normalized.extend_from_slice(&0u16.to_le_bytes());
    normalized.extend_from_slice(&data[..2]);
    normalized.extend_from_slice(&0u16.to_le_bytes());
    normalized.extend_from_slice(&data[2..]);
    Ok(Some(normalized))
}

fn validate_windowing_orders_support(
    orders: &WindowingOrdersUpdate<'_>,
    window_support_level: WindowSupportLevel,
) -> SessionResult<()> {
    if window_support_level == WindowSupportLevel::SupportedEx
        || !orders.orders.iter().any(|order| order.requires_extended_support())
    {
        return Ok(());
    }

    Err(SessionError::general(
        "received extended window order fields without negotiated extended window support",
    ))
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

/// Apply every compositor delta to `image` and return the single region covering them.
///
/// Emitting one update per delta would be correct but ruinous: a consumer is entitled to
/// redraw whatever a `GraphicsUpdate` names, and `ironrdp-client` rebuilds the entire
/// framebuffer for each one, so an N-rectangle frame would copy the whole desktop N
/// times. A single SolidFill or CacheToSurface can name up to `u16::MAX` rectangles, so
/// N is the server's choice, not ours. The union's worst case is the full desktop, which
/// is still one copy rather than N.
#[cfg_attr(feature = "__test", visibility::make(pub))]
fn composite_graphics_updates(
    image: &mut DecodedImage,
    updates: impl IntoIterator<Item = (ExclusiveRectangle, Vec<u8>)>,
) -> SessionResult<Option<InclusiveRectangle>> {
    let mut dirty: Option<InclusiveRectangle> = None;
    for (region, data) in updates {
        // egfx maps regions with exclusive right/bottom; the session's InclusiveRectangle
        // is one-past-inclusive. Compositor updates are always non-empty, so the
        // saturating decrements never underflow a real region.
        let region = InclusiveRectangle {
            left: region.left,
            top: region.top,
            right: region.right.saturating_sub(1),
            bottom: region.bottom.saturating_sub(1),
        };

        // `apply_rgba32` reports rejection by returning `InclusiveRectangle::empty()`,
        // which is `(0, 0, 0, 0)` and not distinguishable from a real 1x1 update at the
        // origin. Checking fit here first, rather than branching on that return value,
        // means the delta is skipped outright rather than folded into the accumulator
        // as a phantom region. A successful ResetGraphics resizes `image` to the
        // compositor output before deltas are drained; this guard remains for deltas
        // received before the first reset and future accounting mismatches.
        let fits = region.left <= region.right
            && region.top <= region.bottom
            && region.right < image.width()
            && region.bottom < image.height();
        if !fits {
            warn!(
                ?region,
                image_width = image.width(),
                image_height = image.height(),
                "Dropping a compositor delta outside the image bounds"
            );
            continue;
        }

        let applied = image.apply_rgba32(&data, &region, false)?;
        dirty = Some(match dirty {
            Some(acc) => acc.union(&applied),
            None => applied,
        });
    }
    Ok(dirty)
}

#[cfg(test)]
mod tests {
    use core::any::TypeId;

    use super::*;
    use ironrdp_core::{Decode as _, encode_vec};
    use ironrdp_displaycontrol::pdu::{DisplayControlCapabilities, DisplayControlPdu};
    use ironrdp_dvc::pdu::{
        CreateRequestPdu, DataPdu, DrdynvcDataPdu, DrdynvcServerPdu, SoftSyncChannelList, SoftSyncRequestPdu,
    };
    use ironrdp_pdu::gcc::MonitorFlags;
    use ironrdp_pdu::input::MousePdu;
    use ironrdp_pdu::input::fast_path::KeyboardFlags;
    use ironrdp_pdu::input::mouse::PointerFlags;
    use ironrdp_pdu::pointer::{ColorPointerAttribute, Point16, PointerAttribute, PointerUpdateData};
    use ironrdp_rdpei::pdu::{PenEventPdu, RdpInputProtocolVersion, RdpeiPdu, ScReadyPdu, TouchEventPdu};

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
    fn multitransport_response_encoder_is_exposed_through_active_stage() {
        let stage = ActiveStageBuilder {
            static_channels: StaticChannelSet::new(),
            user_channel_id: 1001,
            io_channel_id: 1003,
            message_channel_id: Some(1004),
            share_id: 1,
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        let response = MultitransportResponsePdu::success(42);

        assert_eq!(
            stage
                .encode_multitransport_response(&response)
                .expect("encode through ActiveStage"),
            stage
                .x224_processor
                .encode_multitransport_response(&response)
                .expect("encode through X.224 processor")
        );
    }

    #[test]
    fn fastpath_input_splits_at_event_limit() {
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
        let events = (0..=u8::MAX)
            .map(|code| FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), u16::from(code)))
            .collect::<Vec<_>>();

        let output = stage.process_fastpath_input(&mut image, &events).unwrap();
        let input_frames = output
            .into_iter()
            .map(|output| {
                let ActiveStageOutput::ResponseFrame(frame) = output else {
                    panic!("expected a fast-path input frame");
                };
                FastPathInput::decode(&mut ReadCursor::new(&frame)).unwrap()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            input_frames
                .iter()
                .map(|input| input.input_events().len())
                .collect::<Vec<_>>(),
            [FastPathInput::MAX_EVENTS, 1]
        );
        assert_eq!(
            input_frames
                .iter()
                .flat_map(FastPathInput::input_events)
                .collect::<Vec<_>>(),
            events.iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn fastpath_input_renders_the_last_mouse_position_in_a_batch() {
        let mut stage = ActiveStageBuilder {
            static_channels: StaticChannelSet::new(),
            user_channel_id: 1001,
            io_channel_id: 1003,
            message_channel_id: None,
            share_id: 1,
            compression_type: None,
            enable_server_pointer: true,
            pointer_software_rendering: true,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 8, 8);
        image
            .update_pointer(Arc::new(DecodedPointer {
                width: 1,
                height: 1,
                hotspot_x: 0,
                hotspot_y: 0,
                bitmap_data: vec![0xff; 4],
            }))
            .expect("set software pointer");
        let mouse_move = |x, y| {
            FastPathInputEvent::MouseEvent(MousePdu {
                flags: PointerFlags::MOVE,
                number_of_wheel_rotation_units: 0,
                x_position: x,
                y_position: y,
            })
        };

        let output = stage
            .process_fastpath_input(&mut image, &[mouse_move(1, 1), mouse_move(5, 5)])
            .expect("process batched mouse input");
        let region = output
            .iter()
            .find_map(|output| match output {
                ActiveStageOutput::GraphicsUpdate(region) => Some(region),
                _ => None,
            })
            .expect("software pointer movement redraw");
        assert_eq!(
            *region,
            InclusiveRectangle {
                left: 0,
                top: 0,
                right: 5,
                bottom: 5,
            }
        );
    }

    #[test]
    fn monitor_layout_is_forwarded_from_x224() {
        let monitors = vec![Monitor {
            left: 0,
            top: 0,
            right: 799,
            bottom: 599,
            flags: MonitorFlags::PRIMARY,
        }];

        let output = ActiveStageOutput::try_from(x224::ProcessorOutput::MonitorLayout(monitors.clone()))
            .expect("monitor layout should be forwarded from X.224");

        let ActiveStageOutput::MonitorLayout(actual) = output else {
            panic!("expected a monitor layout output");
        };
        assert_eq!(actual, monitors);
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

        let (palette_updates, windowing_orders) =
            process_slow_path_graphics(&mut processor, &mut image, None, &palette_data)
                .expect("slow-path palette update should succeed");
        assert!(palette_updates.is_empty());
        assert!(windowing_orders.is_none());

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

    fn window_order(fields_present: u32) -> Vec<u8> {
        let mut order = Vec::new();
        order.push(0x2e);
        let client_area_size = (fields_present & 0x0001_0000 != 0).then_some([0; 8]);
        let order_size: u16 = if client_area_size.is_some() { 19 } else { 11 };
        order.extend_from_slice(&order_size.to_le_bytes());
        order.extend_from_slice(&fields_present.to_le_bytes());
        order.extend_from_slice(&7u32.to_le_bytes());
        if let Some(client_area_size) = client_area_size {
            order.extend_from_slice(&client_area_size);
        }
        order
    }

    fn slow_path_orders_update(order: &[u8]) -> Vec<u8> {
        let mut update = Vec::new();
        update.extend_from_slice(&0u16.to_le_bytes());
        update.extend_from_slice(&0u16.to_le_bytes());
        update.extend_from_slice(&1u16.to_le_bytes());
        update.extend_from_slice(&0u16.to_le_bytes());
        update.extend_from_slice(order);
        update
    }

    #[test]
    fn slow_path_windowing_orders_require_negotiated_support() {
        let mut processor = fast_path::ProcessorBuilder {
            io_channel_id: 0,
            user_channel_id: 0,
            share_id: 0,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 1, 1);
        let update = slow_path_orders_update(&window_order(0x2100_0000));

        let (_, orders) = process_slow_path_graphics(&mut processor, &mut image, None, &update).unwrap();
        assert!(orders.is_none());

        let (_, orders) =
            process_slow_path_graphics(&mut processor, &mut image, Some(WindowSupportLevel::Supported), &update)
                .unwrap();
        assert_eq!(orders.as_deref(), Some(update.as_slice()));
    }

    #[test]
    fn fast_path_windowing_orders_are_normalized_for_forwarding() {
        let order = window_order(0x2100_0000);
        let mut update = Vec::new();
        update.extend_from_slice(&1u16.to_le_bytes());
        update.extend_from_slice(&order);

        let normalized = process_fast_path_windowing_orders(Some(WindowSupportLevel::Supported), &update)
            .unwrap()
            .unwrap();
        assert_eq!(
            normalized,
            [
                0, 0, // updateType
                0, 0, // pad2OctetsA
                1, 0, // numberOrders
                0, 0, // pad2OctetsB
                0x2e, 11, 0, 0, 0, 0, 0x21, 7, 0, 0, 0,
            ]
        );
    }

    #[test]
    fn extended_windowing_orders_require_extended_support() {
        let update = slow_path_orders_update(&window_order(0x0101_0000));
        let mut processor = fast_path::ProcessorBuilder {
            io_channel_id: 0,
            user_channel_id: 0,
            share_id: 0,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 1, 1);

        assert!(
            process_slow_path_graphics(&mut processor, &mut image, Some(WindowSupportLevel::Supported), &update)
                .is_err()
        );
        assert!(
            process_slow_path_graphics(
                &mut processor,
                &mut image,
                Some(WindowSupportLevel::SupportedEx),
                &update
            )
            .is_ok()
        );
    }

    #[test]
    fn prepared_dvc_batches_preserve_channel_and_unframed_messages() {
        let mut stage = active_stage_with_ready_dvcs();

        let resize = stage.prepare_resize(1024, 768, None, None).unwrap().unwrap();
        assert_prepared_batch(&resize, 1);

        let touch = stage
            .prepare_rdpei_touch(TouchEventPdu::new(0, Vec::new()))
            .unwrap()
            .unwrap();
        assert_prepared_batch(&touch, 2);

        let dismiss = stage.prepare_rdpei_dismiss_hovering(3).unwrap().unwrap();
        assert_prepared_batch(&dismiss, 2);

        let pen = stage
            .prepare_rdpei_pen(PenEventPdu::new(0, Vec::new()))
            .unwrap()
            .unwrap();
        assert_prepared_batch(&pen, 2);
    }

    #[test]
    fn tcp_wrappers_encode_prepared_dvc_batches() {
        let mut stage = active_stage_with_ready_dvcs();

        let resize = stage.prepare_resize(1024, 768, None, None).unwrap().unwrap();
        let expected_resize = stage.encode_dvc_messages(resize.into_messages()).unwrap();
        assert_eq!(
            stage.encode_resize(1024, 768, None, None).unwrap().unwrap(),
            expected_resize
        );

        let touch = stage
            .prepare_rdpei_touch(TouchEventPdu::new(0, Vec::new()))
            .unwrap()
            .unwrap();
        let expected_touch = stage.encode_dvc_messages(touch.into_messages()).unwrap();
        assert_eq!(
            stage
                .encode_rdpei_touch(TouchEventPdu::new(0, Vec::new()))
                .unwrap()
                .unwrap(),
            expected_touch
        );

        let dismiss = stage.prepare_rdpei_dismiss_hovering(3).unwrap().unwrap();
        let expected_dismiss = stage.encode_dvc_messages(dismiss.into_messages()).unwrap();
        assert_eq!(
            stage.encode_rdpei_dismiss_hovering(3).unwrap().unwrap(),
            expected_dismiss
        );

        let pen = stage
            .prepare_rdpei_pen(PenEventPdu::new(0, Vec::new()))
            .unwrap()
            .unwrap();
        let expected_pen = stage.encode_dvc_messages(pen.into_messages()).unwrap();
        assert_eq!(
            stage
                .encode_rdpei_pen(PenEventPdu::new(0, Vec::new()))
                .unwrap()
                .unwrap(),
            expected_pen
        );
    }

    #[test]
    fn active_stage_exposes_and_validates_soft_sync_routing() {
        let mut stage = active_stage_with_ready_dvcs();
        stage.enable_reliable_udp_dvc_tunnel().unwrap();
        stage.disable_reliable_udp_dvc_tunnel().unwrap();

        let soft_sync = DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![SoftSyncChannelList::new(
            SoftSyncTunnelType::RELIABLE_UDP,
            vec![1, 2],
        )]));
        let payload = encode_vec(&soft_sync).unwrap();
        assert!(
            stage
                .get_svc_processor_mut::<DrdynvcClient>()
                .unwrap()
                .process(&payload)
                .is_err()
        );
        assert!(!stage.reliable_udp_dvc_tunnel_in_use());
        assert_eq!(stage.dvc_tunnel_for_channel(2), None);

        stage.enable_reliable_udp_dvc_tunnel().unwrap();
        process_drdynvc_pdu(stage.get_svc_processor_mut::<DrdynvcClient>().unwrap(), soft_sync);

        assert_eq!(stage.dvc_tunnel_for_channel(2), Some(SoftSyncTunnelType::RELIABLE_UDP));
        assert!(stage.reliable_udp_dvc_tunnel_in_use());
        assert!(stage.encode_resize(1024, 768, None, None).unwrap().is_err());
        assert!(
            stage
                .encode_rdpei_touch(TouchEventPdu::new(0, Vec::new()))
                .unwrap()
                .is_err()
        );
        assert!(stage.encode_rdpei_dismiss_hovering(3).unwrap().is_err());
        assert!(
            stage
                .encode_rdpei_pen(PenEventPdu::new(0, Vec::new()))
                .unwrap()
                .is_err()
        );

        let rdpei_ready = encode_vec(&RdpeiPdu::ScReady(ScReadyPdu::new(RdpInputProtocolVersion::V200))).unwrap();
        let tunnel_data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
            2,
            rdpei_ready,
        ))))
        .unwrap();
        assert!(
            stage
                .process_dvc_tunnel(SoftSyncTunnelType::LOSSY_UDP, &tunnel_data)
                .is_err()
        );

        let response = stage
            .process_dvc_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &tunnel_data)
            .unwrap();
        assert_prepared_batch(&response, 2);
    }

    fn active_stage_with_ready_dvcs() -> ActiveStage {
        let mut drdynvc = DrdynvcClient::new()
            .with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())))
            .with_dynamic_channel(RdpeiClient::default());

        process_drdynvc_pdu(
            &mut drdynvc,
            DrdynvcServerPdu::Create(CreateRequestPdu::new(
                1,
                ironrdp_displaycontrol::CHANNEL_NAME.to_owned(),
            )),
        );
        process_drdynvc_pdu(
            &mut drdynvc,
            DrdynvcServerPdu::Create(CreateRequestPdu::new(2, ironrdp_rdpei::CHANNEL_NAME.to_owned())),
        );

        let display_caps = DisplayControlPdu::Caps(DisplayControlCapabilities::new(1, 3840, 2400).unwrap());
        process_drdynvc_pdu(
            &mut drdynvc,
            DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
                1,
                encode_vec(&display_caps).unwrap(),
            ))),
        );
        process_drdynvc_pdu(
            &mut drdynvc,
            DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
                2,
                encode_vec(&RdpeiPdu::ScReady(ScReadyPdu::new(RdpInputProtocolVersion::V200))).unwrap(),
            ))),
        );

        let mut static_channels = StaticChannelSet::new();
        assert!(static_channels.insert(drdynvc).is_none());
        assert!(
            static_channels
                .attach_channel_id(TypeId::of::<DrdynvcClient>(), 1004)
                .is_none()
        );

        ActiveStageBuilder {
            static_channels,
            user_channel_id: 1001,
            io_channel_id: 1003,
            message_channel_id: None,
            share_id: 1,
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build()
    }

    fn process_drdynvc_pdu(client: &mut DrdynvcClient, pdu: DrdynvcServerPdu) {
        let payload = encode_vec(&pdu).unwrap();
        client.process(&payload).unwrap();
    }

    fn assert_prepared_batch(batch: &DvcMessageBatch, expected_channel_id: u32) {
        assert_eq!(batch.channel_id(), expected_channel_id);
        assert!(!batch.messages().is_empty());
        assert!(
            batch
                .messages()
                .iter()
                .all(|message| !message.encode_unframed_pdu().unwrap().is_empty())
        );
    }
}
