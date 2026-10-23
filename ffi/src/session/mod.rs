pub mod image;

fn active_stage_output(output: ironrdp::session::ActiveStageOutput) -> ffi::ActiveStageOutput {
    let encoded_windowing_orders = match &output {
        ironrdp::session::ActiveStageOutput::WindowingOrders(orders) => {
            Some(encode_slow_path_orders_update(orders).map_err(|error| error.to_string()))
        }
        _ => None,
    };
    ffi::ActiveStageOutput(output, encoded_windowing_orders)
}

/// Encodes windowing orders as a complete slow-path Orders update ([MS-RDPBCGR] 2.2.9.1.1.3.1
/// Slow-Path Graphics Update carrying an [MS-RDPEGDI] 2.2.2.1 Orders Update), the byte form this
/// API exposed before orders were decoded.
///
/// Orders the session decoded always encode; see
/// [`encode_slow_path_windowing_orders_update`](ironrdp::pdu::window::encode_slow_path_windowing_orders_update).
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr
/// [MS-RDPEGDI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegdi
fn encode_slow_path_orders_update(
    orders: &[ironrdp::pdu::window::WindowingOrder],
) -> ironrdp::core::EncodeResult<Vec<u8>> {
    let mut update = vec![0; ironrdp::pdu::window::slow_path_windowing_orders_update_size(orders)];
    ironrdp::pdu::window::encode_slow_path_windowing_orders_update(
        orders,
        &mut ironrdp::core::WriteCursor::new(&mut update),
    )?;
    Ok(update)
}

#[diplomat::bridge]
pub mod ffi {
    use std::collections::VecDeque;

    use super::image::ffi::DecodedImage;
    use crate::clipboard::message::ffi::{
        ClipboardFormatId, ClipboardFormatIterator, ClipboardMessage, FormatDataResponse,
    };
    use crate::connector::activation::ffi::ConnectionActivationSequence;
    use crate::connector::result::ffi::ConnectionResult;
    use crate::dvc::dvc_pipe_proxy_message_queue::ffi::DvcPipeProxyMessage;
    use crate::error::ffi::IronRdpError;
    use crate::error::{IncorrectEnumTypeError, ValueConsumedError};
    use crate::graphics::ffi::DecodedPointer;
    use crate::pdu::ffi::{Action, FastPathInputEventIterator, InclusiveRectangle};
    use crate::utils::ffi::{BytesSlice, Position, VecU8};

    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque]
    pub struct ActiveStage(
        pub ironrdp::session::ActiveStage,
        pub ironrdp::connector::connection_activation::ConnectionActivationFactory,
    );

    /// The second field holds windowing orders re-encoded as a slow-path Orders update,
    /// so the byte-oriented getter can borrow them.
    #[diplomat::opaque]
    pub struct ActiveStageOutput(
        pub ironrdp::session::ActiveStageOutput,
        pub Option<Result<Vec<u8>, String>>,
    );

    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque]
    pub struct ActiveStageOutputIterator(pub VecDeque<ironrdp::session::ActiveStageOutput>);

    #[diplomat::opaque]
    pub struct MonitorLayoutIterator(pub VecDeque<ironrdp::pdu::gcc::Monitor>);

    #[diplomat::opaque]
    pub struct MonitorLayoutEntry(pub ironrdp::pdu::gcc::Monitor);

    impl ActiveStageOutputIterator {
        pub fn len(&self) -> usize {
            self.0.len()
        }

        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        pub fn next(&mut self) -> Option<Box<ActiveStageOutput>> {
            self.0.pop_front().map(super::active_stage_output).map(Box::new)
        }
    }

    impl MonitorLayoutIterator {
        pub fn len(&self) -> usize {
            self.0.len()
        }

        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        pub fn next(&mut self) -> Option<Box<MonitorLayoutEntry>> {
            self.0.pop_front().map(MonitorLayoutEntry).map(Box::new)
        }
    }

    impl MonitorLayoutEntry {
        #[diplomat::attr(auto, getter = "left")]
        pub fn get_left(&self) -> i32 {
            self.0.left
        }

        #[diplomat::attr(auto, getter = "top")]
        pub fn get_top(&self) -> i32 {
            self.0.top
        }

        #[diplomat::attr(auto, getter = "right")]
        pub fn get_right(&self) -> i32 {
            self.0.right
        }

        #[diplomat::attr(auto, getter = "bottom")]
        pub fn get_bottom(&self) -> i32 {
            self.0.bottom
        }

        pub fn is_primary(&self) -> bool {
            self.0.flags.contains(ironrdp::pdu::gcc::MonitorFlags::PRIMARY)
        }
    }

    impl ActiveStage {
        pub fn new(connection_result: &mut ConnectionResult) -> Result<Box<Self>, Box<IronRdpError>> {
            let connection_result = connection_result
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("connection_result"))?;

            // Retain the factory to drive the Deactivation-Reactivation Sequence.
            let activation_factory = connection_result.activation_factory;

            let window_support_level = connection_result.window_support_level;
            let mut stage = ironrdp::session::ActiveStageBuilder {
                static_channels: connection_result.static_channels,
                user_channel_id: connection_result.user_channel_id,
                io_channel_id: connection_result.io_channel_id,
                message_channel_id: connection_result.message_channel_id,
                share_id: connection_result.share_id,
                compression_type: connection_result.compression_type,
                enable_server_pointer: connection_result.enable_server_pointer,
                pointer_software_rendering: connection_result.pointer_software_rendering,
            }
            .build();
            stage.set_window_support_level(window_support_level);

            Ok(Box::new(ActiveStage(stage, activation_factory)))
        }

        /// Produces a fresh connection activation sequence to drive the Deactivation-Reactivation
        /// Sequence.
        ///
        /// Call this upon receiving a [`ActiveStageOutputType::DeactivateAll`] output, drive the
        /// returned sequence until it is finalized, then discard it.
        pub fn create_connection_activation(&self) -> Box<ConnectionActivationSequence> {
            Box::new(ConnectionActivationSequence(Box::new(self.1.create())))
        }

        pub fn process(
            &mut self,
            image: &mut DecodedImage,
            action: &Action,
            payload: &[u8],
        ) -> Result<Box<ActiveStageOutputIterator>, Box<IronRdpError>> {
            // The .NET host calls this right after reading each frame, so the call time stands in
            // for the arrival time.
            let mut outputs = self
                .0
                .process(&mut image.0, action.0, payload, Some(crate::utils::monotonic_now()))?;
            // After DeactivateAll the frame would carry pre-reactivation channel IDs; after
            // Terminate there is no session left. The next poll flushes them instead.
            let session_continues = !outputs.iter().any(|output| {
                matches!(
                    output,
                    ironrdp::session::ActiveStageOutput::DeactivateAll
                        | ironrdp::session::ActiveStageOutput::Terminate(_)
                )
            });
            if session_continues {
                outputs.extend(self.poll_rdpdr_deferred_output()?);
            }
            Ok(Box::new(ActiveStageOutputIterator(outputs.into())))
        }

        /// Collects RDPDR completions that finished asynchronously (e.g. smartcard waits).
        ///
        /// [`Self::process`] already collects them after each frame. Also call this on a short
        /// timer (e.g. every 50 ms) so that they reach the server while it sends no frames. Do
        /// not call it concurrently with other methods of this `ActiveStage`.
        pub fn poll_rdpdr_deferred(&mut self) -> Result<Box<ActiveStageOutputIterator>, Box<IronRdpError>> {
            let outputs = self.poll_rdpdr_deferred_output()?;
            Ok(Box::new(ActiveStageOutputIterator(outputs.into())))
        }

        pub fn process_fastpath_input(
            &mut self,
            image: &mut DecodedImage,
            fastpath_input: &FastPathInputEventIterator,
        ) -> Result<Box<ActiveStageOutputIterator>, Box<IronRdpError>> {
            Ok(self
                .0
                .process_fastpath_input(&mut image.0, &fastpath_input.0)
                .map(|outputs| Box::new(ActiveStageOutputIterator(outputs.into())))?)
        }

        pub fn initiate_clipboard_copy(
            &mut self,
            formats: &ClipboardFormatIterator,
        ) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let formats = formats.0.clone();
            let clipboard = self
                .0
                .get_svc_processor_mut::<ironrdp::cliprdr::CliprdrClient>()
                .ok_or("clipboard svc processor not found in active stage")?;

            let result = clipboard.initiate_copy(&formats)?;

            let frame = self.0.process_svc_processor_messages(result)?;

            Ok(Box::new(VecU8(frame)))
        }

        pub fn initiate_clipboard_paste(
            &mut self,
            format_id: &ClipboardFormatId,
        ) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let format_id = format_id.0;
            let clipboard = self
                .0
                .get_svc_processor_mut::<ironrdp::cliprdr::CliprdrClient>()
                .ok_or("clipboard svc processor not found in active stage")?;

            let result = clipboard.initiate_paste(format_id)?;

            let frame = self.0.process_svc_processor_messages(result)?;

            Ok(Box::new(VecU8(frame)))
        }

        pub fn submit_clipboard_format_data(
            &mut self,
            format_data_response: &mut FormatDataResponse,
        ) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let data = format_data_response
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("format_data_response"))?;
            let clipboard = self
                .0
                .get_svc_processor::<ironrdp::cliprdr::CliprdrClient>()
                .ok_or("clipboard svc processor not found in active stage")?;

            let result = clipboard.submit_format_data(data)?;

            let frame = self.0.process_svc_processor_messages(result)?;

            Ok(Box::new(VecU8(frame)))
        }

        /// Hands any message of the clipboard backend (e.g. `WinCliprdr`) to the clipboard
        /// channel, file transfer messages included, and returns the frame to send.
        ///
        /// The frame is empty when there is nothing to send. A backend `Error` message is
        /// returned as an error. Errors only concern that message: the session can go on.
        pub fn submit_clipboard_message(
            &mut self,
            message: &ClipboardMessage,
        ) -> Result<Box<VecU8>, Box<IronRdpError>> {
            use ironrdp::cliprdr::backend::ClipboardMessage as Message;

            let clipboard = self
                .0
                .get_svc_processor_mut::<ironrdp::cliprdr::CliprdrClient>()
                .ok_or("clipboard svc processor not found in active stage")?;

            let result = match &message.0 {
                Message::SendInitiateCopy(formats) => clipboard.initiate_copy(formats)?,
                Message::SendInitiateFileCopy(files) => clipboard.initiate_file_copy(files.clone())?,
                Message::SendFormatData(response) => clipboard.submit_format_data(response.clone())?,
                Message::SendInitiatePaste(format) => clipboard.initiate_paste(*format)?,
                Message::SendFileContentsRequest(request) => clipboard.request_file_contents(request.clone())?,
                Message::SendFileContentsResponse(response) => clipboard.submit_file_contents(response.clone())?,
                Message::Error(error) => {
                    let error: &dyn ironrdp::cliprdr::backend::ClipboardError = error.as_ref();
                    return Err(error.into());
                }
            };

            let frame = self.0.process_svc_processor_messages(result)?;

            Ok(Box::new(VecU8(frame)))
        }

        /// Expires clipboard locks and file transfers that timed out, returning the frame to send
        /// (empty when there is nothing to send).
        ///
        /// Call it every few seconds while file transfer is enabled on the clipboard backend.
        pub fn drive_clipboard_timeouts(&mut self) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let clipboard = self
                .0
                .get_svc_processor_mut::<ironrdp::cliprdr::CliprdrClient>()
                .ok_or("clipboard svc processor not found in active stage")?;

            let result = clipboard.drive_timeouts()?;

            let frame = self.0.process_svc_processor_messages(result)?;

            Ok(Box::new(VecU8(frame)))
        }

        pub fn send_dvc_pipe_proxy_message(
            &mut self,
            message: &mut DvcPipeProxyMessage,
        ) -> Result<Box<VecU8>, Box<IronRdpError>> {
            let messages = core::mem::take(&mut message.0.1);

            if messages.is_empty() {
                return Err("no dvc messages to send (message sent twice?)".into());
            }

            let frame = self.0.encode_dvc_messages(messages)?;
            Ok(Box::new(VecU8(frame)))
        }

        pub fn graceful_shutdown(&mut self) -> Result<Box<ActiveStageOutputIterator>, Box<IronRdpError>> {
            let outputs = self.0.graceful_shutdown()?;
            Ok(Box::new(ActiveStageOutputIterator(outputs.into())))
        }

        pub fn encoded_resize(
            &mut self,
            width: u32,
            height: u32,
        ) -> Result<Option<Box<ActiveStageOutputIterator>>, Box<IronRdpError>> {
            let (width, height) = ironrdp::displaycontrol::pdu::MonitorLayoutEntry::adjust_display_size(width, height);
            Ok(self
                .0
                .encode_resize(width, height, None, Some((width, height)))
                .map(|outputs| {
                    outputs.map(|outputs| {
                        Box::new(ActiveStageOutputIterator(
                            vec![ironrdp::session::ActiveStageOutput::ResponseFrame(outputs)].into(),
                        ))
                    })
                })
                .transpose()?)
        }

        pub fn set_fastpath_processor(
            &mut self,
            io_channel_id: u16,
            user_channel_id: u16,
            share_id: u32,
            enable_server_pointer: bool,
            pointer_software_rendering: bool,
        ) {
            let static_channel_chunk_size = self.0.static_channel_chunk_size();
            debug_assert!(self.0.reactivate(
                io_channel_id,
                user_channel_id,
                share_id,
                enable_server_pointer,
                pointer_software_rendering,
                static_channel_chunk_size,
            ));
        }

        /// Rebuilds active-stage processors for a Deactivation-Reactivation Sequence.
        ///
        /// This retains negotiated bulk compression and applies the refreshed server-pointer state
        /// and static channel chunk size.
        #[expect(
            clippy::too_many_arguments,
            reason = "the C-compatible reactivation entry point exposes the negotiated activation fields"
        )]
        pub fn reactivate(
            &mut self,
            io_channel_id: u16,
            user_channel_id: u16,
            share_id: u32,
            enable_server_pointer: bool,
            pointer_software_rendering: bool,
            static_channel_chunk_size: usize,
            window_support_level: i8,
        ) -> Result<(), Box<IronRdpError>> {
            let window_support_level = match window_support_level {
                -1 => None,
                1 => Some(ironrdp::pdu::rdp::capability_sets::WindowSupportLevel::Supported),
                2 => Some(ironrdp::pdu::rdp::capability_sets::WindowSupportLevel::SupportedEx),
                _ => return Err("invalid Window List support level".into()),
            };
            if !self.0.reactivate(
                io_channel_id,
                user_channel_id,
                share_id,
                enable_server_pointer,
                pointer_software_rendering,
                static_channel_chunk_size,
            ) {
                return Err("invalid static channel chunk size".into());
            }
            self.0.set_window_support_level(window_support_level);

            Ok(())
        }

        #[diplomat::attr(auto, setter = "enable_server_pointer")]
        pub fn set_enable_server_pointer(&mut self, enable_server_pointer: bool) {
            self.0.set_enable_server_pointer(enable_server_pointer);
        }

        fn poll_rdpdr_deferred_output(
            &mut self,
        ) -> Result<Vec<ironrdp::session::ActiveStageOutput>, Box<IronRdpError>> {
            let Some(rdpdr) = self.0.get_svc_processor_mut::<ironrdp::rdpdr::Rdpdr>() else {
                return Ok(Vec::new());
            };

            let messages = rdpdr.poll_deferred_messages()?;
            if messages.is_empty() {
                return Ok(Vec::new());
            }

            let frame = self
                .0
                .process_svc_messages_by_name(&ironrdp::rdpdr::Rdpdr::NAME, messages)?;
            Ok(vec![ironrdp::session::ActiveStageOutput::ResponseFrame(frame)])
        }
    }

    pub enum ActiveStageOutputType {
        ResponseFrame = 0,
        GraphicsUpdate = 1,
        PointerDefault = 2,
        PointerHidden = 3,
        PointerPosition = 4,
        PointerBitmap = 5,
        Terminate = 6,
        DeactivateAll = 7,
        MultitransportRequest = 8,
        /// Auto-detect network characteristics from server.
        /// Use `get_autodetect_network_characteristics()` to retrieve
        /// RTT and bandwidth values for connection quality monitoring.
        AutoDetect = 9,
        SaveSessionInfo = 10,
        AutoReconnectCookie = 11,
        WindowingOrders = 12,
        AutoReconnectFailed = 13,
        MonitorLayout = 14,
        /// RemoteApp window pixels from graphics pipeline surfaces mapped to RAIL windows.
        /// The pixels are not exposed yet: this binding does not enable RemoteApp,
        /// so servers do not map surfaces to windows.
        WindowSurfaceUpdates = 15,
        /// The server sent a Server Redirection PDU; use `get_server_redirection()` to read the target.
        ServerRedirection = 16,
        LogonErrors = 17,
        /// The ID of the remote session, reported with a completed logon.
        /// Use `get_session_id()` to retrieve it.
        SessionId = 18,
    }

    impl ActiveStageOutput {
        #[diplomat::attr(auto, getter = "enum_type")]
        pub fn get_enum_type(&self) -> ActiveStageOutputType {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::ResponseFrame { .. } => ActiveStageOutputType::ResponseFrame,
                ironrdp::session::ActiveStageOutput::GraphicsUpdate { .. } => ActiveStageOutputType::GraphicsUpdate,
                ironrdp::session::ActiveStageOutput::PointerDefault => ActiveStageOutputType::PointerDefault,
                ironrdp::session::ActiveStageOutput::PointerHidden => ActiveStageOutputType::PointerHidden,
                ironrdp::session::ActiveStageOutput::PointerPosition { .. } => ActiveStageOutputType::PointerPosition,
                ironrdp::session::ActiveStageOutput::PointerBitmap { .. } => ActiveStageOutputType::PointerBitmap,
                ironrdp::session::ActiveStageOutput::WindowingOrders(_) => ActiveStageOutputType::WindowingOrders,
                ironrdp::session::ActiveStageOutput::Terminate { .. } => ActiveStageOutputType::Terminate,
                ironrdp::session::ActiveStageOutput::DeactivateAll => ActiveStageOutputType::DeactivateAll,
                ironrdp::session::ActiveStageOutput::MultitransportRequest { .. } => {
                    ActiveStageOutputType::MultitransportRequest
                }
                ironrdp::session::ActiveStageOutput::AutoDetect { .. } => ActiveStageOutputType::AutoDetect,
                ironrdp::session::ActiveStageOutput::SaveSessionInfo { .. } => ActiveStageOutputType::SaveSessionInfo,
                ironrdp::session::ActiveStageOutput::AutoReconnectCookie { .. } => {
                    ActiveStageOutputType::AutoReconnectCookie
                }
                ironrdp::session::ActiveStageOutput::AutoReconnectFailed => ActiveStageOutputType::AutoReconnectFailed,
                ironrdp::session::ActiveStageOutput::MonitorLayout(_) => ActiveStageOutputType::MonitorLayout,
                ironrdp::session::ActiveStageOutput::WindowSurfaceUpdates(_) => {
                    ActiveStageOutputType::WindowSurfaceUpdates
                }
                ironrdp::session::ActiveStageOutput::LogonErrors(_) => ActiveStageOutputType::LogonErrors,
                ironrdp::session::ActiveStageOutput::SessionId(_) => ActiveStageOutputType::SessionId,
                ironrdp::session::ActiveStageOutput::ServerRedirection(_) => ActiveStageOutputType::ServerRedirection,
            }
        }

        #[diplomat::attr(auto, getter = "response_frame")]
        pub fn get_response_frame(&self) -> Result<Box<BytesSlice<'_>>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::ResponseFrame(frame) => Ok(Box::new(BytesSlice(frame))),
                _ => Err(IncorrectEnumTypeError::on_variant("ResponseFrame")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

        #[diplomat::attr(auto, getter = "graphics_update")]
        pub fn get_graphics_update(&self) -> Result<Box<InclusiveRectangle>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::GraphicsUpdate(rect) => {
                    Ok(Box::new(InclusiveRectangle(rect.clone())))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("GraphicsUpdate")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

        #[diplomat::attr(auto, getter = "pointer_position")]
        pub fn get_pointer_position(&self) -> Result<Position, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::PointerPosition { x, y } => Ok(Position { x: *x, y: *y }),
                _ => Err(IncorrectEnumTypeError::on_variant("PointerPosition")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

        #[diplomat::attr(auto, getter = "pointer_bitmap")]
        pub fn get_pointer_bitmap(&self) -> Result<Box<DecodedPointer>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::PointerBitmap(decoded_pointer) => {
                    Ok(DecodedPointer(std::sync::Arc::clone(decoded_pointer)))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("PointerBitmap")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
            .map(Box::new)
        }

        #[diplomat::attr(auto, getter = "windowing_orders")]
        pub fn get_windowing_orders(&self) -> Result<Box<BytesSlice<'_>>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::WindowingOrders(_) => match &self.1 {
                    Some(Ok(update)) => Ok(Box::new(BytesSlice(update))),
                    Some(Err(error)) => Err(error.as_str().into()),
                    None => Err("windowing orders were not encoded".into()),
                },
                _ => Err(IncorrectEnumTypeError::on_variant("WindowingOrders")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

        #[diplomat::attr(auto, getter = "monitor_layout")]
        pub fn get_monitor_layout(&self) -> Result<Box<MonitorLayoutIterator>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::MonitorLayout(monitors) => {
                    Ok(Box::new(MonitorLayoutIterator(monitors.clone().into())))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("MonitorLayout")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

        #[diplomat::attr(auto, getter = "session_id")]
        pub fn get_session_id(&self) -> Result<u32, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::SessionId(session_id) => Ok(*session_id),
                _ => Err(IncorrectEnumTypeError::on_variant("SessionId")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

        #[diplomat::attr(auto, getter = "server_redirection")]
        pub fn get_server_redirection(
            &self,
        ) -> Result<Box<crate::connector::result::ffi::ServerRedirection>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::ServerRedirection(redirection) => Ok(Box::new(
                    crate::connector::result::ffi::ServerRedirection(redirection.as_ref().clone()),
                )),
                _ => Err(IncorrectEnumTypeError::on_variant("ServerRedirection")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

        #[diplomat::attr(auto, getter = "terminate")]
        pub fn get_terminate(&self) -> Result<Box<GracefulDisconnectReason>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::Terminate(reason) => Ok(GracefulDisconnectReason(reason.clone())),
                _ => Err(IncorrectEnumTypeError::on_variant("Terminate")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
            .map(Box::new)
        }

        /// Returns the multitransport request ID and requested protocol.
        ///
        /// The security cookie is intentionally not exposed — it is sensitive
        /// and only needed internally for transport binding.
        #[expect(
            clippy::as_conversions,
            reason = "RequestedProtocol is #[repr(u16)], cast is lossless"
        )]
        #[diplomat::attr(auto, getter = "multitransport_request")]
        pub fn get_multitransport_request(&self) -> Result<MultitransportRequest, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::MultitransportRequest(pdu) => Ok(MultitransportRequest {
                    request_id: pdu.request_id,
                    requested_protocol: pdu.requested_protocol as u16,
                }),
                _ => Err(IncorrectEnumTypeError::on_variant("MultitransportRequest")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

        /// Connection quality signals from the server's auto-detect mechanism.
        /// Returns RTT and bandwidth measurements for health monitoring.
        /// These values will feed into FramePacingFeedback when the
        /// library-level health observer traits from #1158 land.
        #[diplomat::attr(auto, getter = "autodetect_network_characteristics")]
        pub fn get_autodetect_network_characteristics(&self) -> Result<NetworkCharacteristics, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::AutoDetect(
                    ironrdp::pdu::rdp::autodetect::AutoDetectRequest::NetworkCharacteristicsResult {
                        base_rtt_ms,
                        bandwidth_kbps,
                        average_rtt_ms,
                        ..
                    },
                ) => Ok(NetworkCharacteristics {
                    base_rtt_ms: base_rtt_ms.unwrap_or(0),
                    has_base_rtt: base_rtt_ms.is_some(),
                    average_rtt_ms: *average_rtt_ms,
                    bandwidth_kbps: bandwidth_kbps.unwrap_or(0),
                    has_bandwidth: bandwidth_kbps.is_some(),
                }),
                _ => Err(IncorrectEnumTypeError::on_variant("AutoDetect")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }
    }

    /// Connection quality measurements from server auto-detect (MS-RDPBCGR 2.2.14).
    pub struct NetworkCharacteristics {
        /// Lowest detected round-trip time in milliseconds.
        /// Only valid when `has_base_rtt` is true.
        pub base_rtt_ms: u32,
        pub has_base_rtt: bool,
        /// Current average round-trip time in milliseconds.
        pub average_rtt_ms: u32,
        /// Estimated bandwidth in kilobits per second.
        /// Only valid when `has_bandwidth` is true.
        pub bandwidth_kbps: u32,
        pub has_bandwidth: bool,
    }

    pub struct MultitransportRequest {
        pub request_id: u32,
        pub requested_protocol: u16,
    }

    #[diplomat::opaque]
    pub struct GracefulDisconnectReason(pub ironrdp::session::GracefulDisconnectReason);
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use ironrdp::pdu::gcc::{Monitor, MonitorFlags};

    use super::ffi::{ActiveStageOutputIterator, ActiveStageOutputType};

    #[test]
    fn active_stage_output_iterator_preserves_response_frame_order() {
        let mut iterator = ActiveStageOutputIterator(VecDeque::from([
            ironrdp::session::ActiveStageOutput::ResponseFrame(vec![1]),
            ironrdp::session::ActiveStageOutput::ResponseFrame(vec![2]),
        ]));

        let first = iterator.next().unwrap();
        let ironrdp::session::ActiveStageOutput::ResponseFrame(first) = &first.0 else {
            panic!("expected a response frame");
        };
        assert_eq!(first, &[1]);

        let second = iterator.next().unwrap();
        let ironrdp::session::ActiveStageOutput::ResponseFrame(second) = &second.0 else {
            panic!("expected a response frame");
        };
        assert_eq!(second, &[2]);

        assert!(iterator.next().is_none());
    }

    // oracle: spec MS-RDPERP §2.2.1.3.2.2.1 (Version 1 is not a valid notification icon version)
    #[test]
    fn windowing_orders_that_cannot_be_encoded_report_an_error() {
        use ironrdp::pdu::window::{NotifyIconOrder, NotifyIconState, WindowingOrder};

        let output = super::active_stage_output(ironrdp::session::ActiveStageOutput::WindowingOrders(vec![
            WindowingOrder::NotifyIcon(NotifyIconOrder {
                window_id: 7,
                notify_icon_id: 1,
                is_new: false,
                state: Some(Box::new(NotifyIconState {
                    version: Some(1),
                    ..NotifyIconState::default()
                })),
            }),
        ]));

        assert!(output.get_windowing_orders().is_err());
    }

    // oracle: spec MS-RDPBCGR §2.2.9.1.1.3.1, MS-RDPEGDI §2.2.2.1 and MS-RDPERP §2.2.1.3.1.2.4
    #[test]
    fn active_stage_output_exposes_windowing_orders_as_a_slow_path_update() {
        use ironrdp::pdu::window::{WindowOrder, WindowOrderBody, WindowingOrder};

        let output = super::active_stage_output(ironrdp::session::ActiveStageOutput::WindowingOrders(vec![
            WindowingOrder::Window(WindowOrder {
                window_id: 7,
                is_new: false,
                body: WindowOrderBody::Deleted,
            }),
        ]));

        let Ok(update) = output.get_windowing_orders() else {
            panic!("windowing orders should be accessible");
        };
        assert_eq!(
            update.0,
            [
                0, 0, // updateType: UPDATETYPE_ORDERS
                0, 0, // pad2OctetsA
                1, 0, // numberOrders
                0, 0, // pad2OctetsB
                0x2e, 11, 0, // controlFlags, orderSize
                0, 0, 0, 0x21, // fieldsPresent: WINDOW | DELETED
                7, 0, 0, 0, // WindowId
            ]
        );
    }

    #[test]
    fn active_stage_output_exposes_monitor_layout() {
        let output = super::active_stage_output(ironrdp::session::ActiveStageOutput::MonitorLayout(vec![Monitor {
            left: -800,
            top: 0,
            right: -1,
            bottom: 599,
            flags: MonitorFlags::PRIMARY,
        }]));

        assert!(matches!(output.get_enum_type(), ActiveStageOutputType::MonitorLayout));

        let Ok(mut monitors) = output.get_monitor_layout() else {
            panic!("monitor layout should be accessible");
        };
        let monitor = monitors.next().expect("the monitor layout should contain one monitor");
        assert_eq!(monitor.get_left(), -800);
        assert_eq!(monitor.get_top(), 0);
        assert_eq!(monitor.get_right(), -1);
        assert_eq!(monitor.get_bottom(), 599);
        assert!(monitor.is_primary());
        assert!(monitors.next().is_none());
    }
}
