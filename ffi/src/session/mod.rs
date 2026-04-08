pub mod image;

#[diplomat::bridge]
pub mod ffi {

    use super::image::ffi::DecodedImage;
    use crate::clipboard::message::ffi::{ClipboardFormatId, ClipboardFormatIterator, FormatDataResponse};
    use crate::connector::activation::ffi::ConnectionActivationSequence;
    use crate::connector::result::ffi::ConnectionResult;
    use crate::dvc::dvc_pipe_proxy_message_queue::ffi::DvcPipeProxyMessage;
    use crate::error::ffi::IronRdpError;
    use crate::error::{IncorrectEnumTypeError, ValueConsumedError};
    use crate::graphics::ffi::DecodedPointer;
    use crate::pdu::ffi::{Action, FastPathInputEventIterator, InclusiveRectangle};
    use crate::utils::ffi::{BytesSlice, Position, VecU8};

    #[diplomat::opaque]
    pub struct ActiveStage(pub ironrdp::session::ActiveStage);

    #[diplomat::opaque]
    pub struct ActiveStageOutput(pub ironrdp::session::ActiveStageOutput);

    #[diplomat::opaque]
    pub struct ActiveStageOutputIterator(pub Vec<ironrdp::session::ActiveStageOutput>);

    impl ActiveStageOutputIterator {
        pub fn len(&self) -> usize {
            self.0.len()
        }

        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        pub fn next(&mut self) -> Option<Box<ActiveStageOutput>> {
            self.0.pop().map(ActiveStageOutput).map(Box::new)
        }
    }

    impl ActiveStage {
        pub fn new(connection_result: &mut ConnectionResult) -> Result<Box<Self>, Box<IronRdpError>> {
            Ok(Box::new(ActiveStage(ironrdp::session::ActiveStage::new(
                connection_result
                    .0
                    .take()
                    .ok_or_else(|| ValueConsumedError::for_item("connection_result"))?,
            ))))
        }

        pub fn process(
            &mut self,
            image: &mut DecodedImage,
            action: &Action,
            payload: &[u8],
        ) -> Result<Box<ActiveStageOutputIterator>, Box<IronRdpError>> {
            let outputs = self.0.process(&mut image.0, action.0, payload)?;
            Ok(Box::new(ActiveStageOutputIterator(outputs)))
        }

        pub fn process_fastpath_input(
            &mut self,
            image: &mut DecodedImage,
            fastpath_input: &FastPathInputEventIterator,
        ) -> Result<Box<ActiveStageOutputIterator>, Box<IronRdpError>> {
            Ok(self
                .0
                .process_fastpath_input(&mut image.0, &fastpath_input.0)
                .map(|outputs| Box::new(ActiveStageOutputIterator(outputs)))?)
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
            Ok(Box::new(ActiveStageOutputIterator(outputs)))
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
                        Box::new(ActiveStageOutputIterator(vec![
                            ironrdp::session::ActiveStageOutput::ResponseFrame(outputs),
                        ]))
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
            self.0.set_fastpath_processor(
                ironrdp::session::fast_path::ProcessorBuilder {
                    io_channel_id,
                    user_channel_id,
                    share_id,
                    enable_server_pointer,
                    pointer_software_rendering,
                    bulk_decompressor: None,
                }
                .build(),
            );
            self.0.set_share_id(share_id);
        }

        pub fn set_enable_server_pointer(&mut self, enable_server_pointer: bool) {
            self.0.set_enable_server_pointer(enable_server_pointer);
        }
    }

    pub enum ActiveStageOutputType {
        ResponseFrame,
        GraphicsUpdate,
        PointerDefault,
        PointerHidden,
        PointerPosition,
        PointerBitmap,
        Terminate,
        DeactivateAll,
        MultitransportRequest,
        /// Auto-detect network characteristics from server.
        /// Use `get_autodetect_network_characteristics()` to retrieve
        /// RTT and bandwidth values for connection quality monitoring.
        AutoDetect,
    }

    impl ActiveStageOutput {
        pub fn get_enum_type(&self) -> ActiveStageOutputType {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::ResponseFrame { .. } => ActiveStageOutputType::ResponseFrame,
                ironrdp::session::ActiveStageOutput::GraphicsUpdate { .. } => ActiveStageOutputType::GraphicsUpdate,
                ironrdp::session::ActiveStageOutput::PointerDefault => ActiveStageOutputType::PointerDefault,
                ironrdp::session::ActiveStageOutput::PointerHidden => ActiveStageOutputType::PointerHidden,
                ironrdp::session::ActiveStageOutput::PointerPosition { .. } => ActiveStageOutputType::PointerPosition,
                ironrdp::session::ActiveStageOutput::PointerBitmap { .. } => ActiveStageOutputType::PointerBitmap,
                ironrdp::session::ActiveStageOutput::Terminate { .. } => ActiveStageOutputType::Terminate,
                ironrdp::session::ActiveStageOutput::DeactivateAll { .. } => ActiveStageOutputType::DeactivateAll,
                ironrdp::session::ActiveStageOutput::MultitransportRequest { .. } => {
                    ActiveStageOutputType::MultitransportRequest
                }
                ironrdp::session::ActiveStageOutput::AutoDetect { .. } => ActiveStageOutputType::AutoDetect,
            }
        }

        pub fn get_response_frame(&self) -> Result<Box<BytesSlice<'_>>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::ResponseFrame(frame) => Ok(Box::new(BytesSlice(frame))),
                _ => Err(IncorrectEnumTypeError::on_variant("ResponseFrame")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

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

        pub fn get_pointer_position(&self) -> Result<Position, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::PointerPosition { x, y } => Ok(Position { x: *x, y: *y }),
                _ => Err(IncorrectEnumTypeError::on_variant("PointerPosition")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
        }

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

        pub fn get_terminate(&self) -> Result<Box<GracefulDisconnectReason>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::Terminate(reason) => Ok(GracefulDisconnectReason(reason.clone())),
                _ => Err(IncorrectEnumTypeError::on_variant("Terminate")
                    .of_enum("ActiveStageOutput")
                    .into()),
            }
            .map(Box::new)
        }

        pub fn get_deactivate_all(&self) -> Result<Box<ConnectionActivationSequence>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::DeactivateAll(cas) => {
                    Ok(ConnectionActivationSequence(cas.clone()))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("DeactivateAll")
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
