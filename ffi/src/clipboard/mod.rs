pub mod message;

pub mod windows;

#[diplomat::bridge]
pub mod ffi {

    use ironrdp::cliprdr::Client;

    #[diplomat::opaque]
    pub struct CliprdrBackendFactory(pub Box<dyn ironrdp::cliprdr::backend::CliprdrBackendFactory>);

    impl CliprdrBackendFactory {
        pub fn build_cliprdr(&self) -> Box<Cliprdr> {
            let backend = self.0.build_cliprdr_backend();
            let cliprdr = ironrdp::cliprdr::Cliprdr::new(backend);
            Box::new(Cliprdr(Some(cliprdr)))
        }
    }

    #[diplomat::opaque]
    pub struct Cliprdr(pub Option<ironrdp::cliprdr::Cliprdr<Client>>);

    // #[diplomat::opaque]
    // pub struct CliprdrReference<'a>(pub &'a ironrdp::cliprdr::Cliprdr<Client>);

    // impl CliprdrReference<'_> {
    //     pub fn initiate_copy(
    //         &self,
    //         formats: &ClipboardFormatIterator,
    //     ) -> Result<Box<ClipboardSvgMessage>, Box<IronRdpError>> {
    //         let result = self.0.initiate_copy(&formats.0)?;
    //         let message = ClipboardSvgMessage(Some(result));
    //         Ok(Box::new(message))
    //     }

    //     pub fn initiate_paste(
    //         &self,
    //         format_id: &ClipboardFormatId,
    //     ) -> Result<Box<ClipboardSvgMessage>, Box<IronRdpError>> {
    //         let result = self.0.initiate_paste(format_id.0)?;
    //         let message = ClipboardSvgMessage(Some(result));
    //         Ok(Box::new(message))
    //     }

    //     pub fn submit_format_data(
    //         &self,
    //         ownd_format_data_response: &mut FormatDataResponse,
    //     ) -> Result<Box<ClipboardSvgMessage>, Box<IronRdpError>> {
    //         let Some(data) = ownd_format_data_response.0.take() else {
    //             return Err(ValueConsumedError::for_item("ownd_format_data_response").into());
    //         };
    //         let result = self.0.submit_format_data(data)?;
    //         let message = ClipboardSvgMessage(Some(result));
    //         Ok(Box::new(message))
    //     }
    // }

    #[diplomat::opaque]
    pub struct ClipboardSvgMessage(pub Option<ironrdp::cliprdr::CliprdrSvcMessages<Client>>);
}

#[derive(Debug)]
pub struct FfiClipbarodMessageProxy {
    pub sender: std::sync::mpsc::Sender<ironrdp::cliprdr::backend::ClipboardMessage>,
}

impl ironrdp::cliprdr::backend::ClipboardMessageProxy for FfiClipbarodMessageProxy {
    fn send_clipboard_message(&self, message: ironrdp::cliprdr::backend::ClipboardMessage) {
        if let Err(err) = self.sender.send(message) {
            tracing::error!("Failed to send clipboard message: {:?}", err);
        }
    }
}
