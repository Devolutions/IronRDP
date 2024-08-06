use tracing::error;

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
            error!("Failed to send clipboard message: {:?}", err);
        }
    }
}
