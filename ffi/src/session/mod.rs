pub mod image;

#[diplomat::bridge]
pub mod ffi {

    use crate::{
        connector::ffi::ConnectionActivationSequence, error::ffi::IronRdpError, graphics::ffi::DecodedPointer, pdu::ffi::{Action, InclusiveRectangle}, utils::ffi::{BytesArray, Position}
    };

    use super::image::ffi::DecodedImage;

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

        pub fn next(&mut self) -> Option<Box<ActiveStageOutput>> {
            self.0.pop().map(ActiveStageOutput).map(Box::new)
        }
    }

    impl ActiveStage {
        pub fn process(
            &mut self,
            image: &mut DecodedImage,
            action: &Action,
            payload: &[u8],
        ) -> Result<Box<ActiveStageOutputIterator>, Box<IronRdpError>> {
            let outputs = self.0.process(image.0, action.0, payload)?;
            Ok(Box::new(ActiveStageOutputIterator(outputs)))
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
        pub fn get_type(&self) -> ActiveStageOutputType {
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

        pub fn get_response_frame<'a>(&'a self) -> Option<Box<BytesArray<'a>>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::ResponseFrame(frame) => Some(Box::new(BytesArray(frame))),
                _ => None,
            }
        }

        pub fn get_graphics_update(&self) -> Option<Box<InclusiveRectangle>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::GraphicsUpdate(rect) => {
                    Some(Box::new(InclusiveRectangle(rect.clone())))
                }
                _ => None,
            }
        }

        pub fn get_pointer_position(&self) -> Option<Box<Position>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::PointerPosition { x, y } => Some(Position { x: *x, y: *y }),
                _ => None,
            }
            .map(Box::new)
        }

        pub fn get_pointer_butmap(&self) -> Option<Box<DecodedPointer>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::PointerBitmap(decoded_pointer) => {
                    Some(DecodedPointer(decoded_pointer.clone()))
                }
                _ => None,
            }.map(Box::new)
        }

        pub fn get_terminate(&self) -> Option<Box<GracefulDisconnectReason>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::Terminate(reason) => Some(GracefulDisconnectReason(reason.clone())),
                _ => None,
            }.map(Box::new)
        }

        pub fn get_deactivate_all(&self) -> Option<Box<ConnectionActivationSequence>> {
            match &self.0 {
                ironrdp::session::ActiveStageOutput::DeactivateAll(cas) => Some(ConnectionActivationSequence(cas.clone())),
                _ => None,
            }.map(Box::new)
        }
    }

    #[diplomat::opaque]
    pub struct GracefulDisconnectReason(pub ironrdp::session::GracefulDisconnectReason);
}
