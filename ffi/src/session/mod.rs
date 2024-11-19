pub mod image;

#[diplomat::bridge]
pub mod ffi {

    use super::image::ffi::DecodedImage;
    use crate::clipboard::message::ffi::{ClipboardFormatId, ClipboardFormatIterator, FormatDataResponse};
    use crate::connector::activation::ffi::ConnectionActivationSequence;
    use crate::connector::result::ffi::ConnectionResult;
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
                .get_svc_processor::<ironrdp::cliprdr::CliprdrClient>()
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
                .get_svc_processor::<ironrdp::cliprdr::CliprdrClient>()
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
            no_server_pointer: bool,
            pointer_software_rendering: bool,
        ) {
            self.0.set_fastpath_processor(
                ironrdp::session::fast_path::ProcessorBuilder {
                    io_channel_id,
                    user_channel_id,
                    no_server_pointer,
                    pointer_software_rendering,
                }
                .build(),
            );
        }

        pub fn set_no_server_pointer(&mut self, no_server_pointer: bool) {
            self.0.set_no_server_pointer(no_server_pointer);
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
    }

    impl ActiveStageOutput {
        pub fn get_enum_type(&self) -> ActiveStageOutputType {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::ResponseFrame { .. } => ActiveStageOutputType::ResponseFrame,
                ironrdp::session::ActiveStageOutput::GraphicsUpdate { .. } => ActiveStageOutputType::GraphicsUpdate,
                ironrdp::session::ActiveStageOutput::PointerDefault { .. } => ActiveStageOutputType::PointerDefault,
                ironrdp::session::ActiveStageOutput::PointerHidden { .. } => ActiveStageOutputType::PointerHidden,
                ironrdp::session::ActiveStageOutput::PointerPosition { .. } => ActiveStageOutputType::PointerPosition,
                ironrdp::session::ActiveStageOutput::PointerBitmap { .. } => ActiveStageOutputType::PointerBitmap,
                ironrdp::session::ActiveStageOutput::Terminate { .. } => ActiveStageOutputType::Terminate,
                ironrdp::session::ActiveStageOutput::DeactivateAll { .. } => ActiveStageOutputType::DeactivateAll,
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
                    Ok(DecodedPointer(std::rc::Rc::clone(decoded_pointer)))
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
    }

    #[diplomat::opaque]
    pub struct GracefulDisconnectReason(pub ironrdp::session::GracefulDisconnectReason);
}
