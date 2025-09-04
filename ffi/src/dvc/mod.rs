pub mod dvc_pipe_proxy_message_queue;

#[diplomat::bridge]
pub mod ffi {
    use crate::dvc::dvc_pipe_proxy_message_queue::ffi::DvcPipeProxyMessageSink;

    #[diplomat::opaque]
    pub struct DrdynvcChannel(pub ironrdp::dvc::DrdynvcClient);

    #[diplomat::opaque]
    #[derive(Clone)]
    pub struct DvcPipeProxyDescriptor {
        pub channel_name: String,
        pub pipe_name: String,
    }

    impl DvcPipeProxyDescriptor {
        pub fn new(channel_name: &str, pipe_name: &str) -> Box<Self> {
            Box::new(DvcPipeProxyDescriptor {
                channel_name: channel_name.to_owned(),
                pipe_name: pipe_name.to_owned(),
            })
        }
    }

    #[diplomat::opaque]
    #[derive(Clone)]
    pub struct DvcPipeProxyConfig {
        pub message_sink: DvcPipeProxyMessageSink,
        pub descriptors: Vec<DvcPipeProxyDescriptor>,
    }

    impl DvcPipeProxyConfig {
        pub fn new(message_sink: &DvcPipeProxyMessageSink) -> Box<Self> {
            Box::new(DvcPipeProxyConfig {
                message_sink: message_sink.clone(),
                descriptors: Vec::new(),
            })
        }

        pub fn add_pipe_proxy(&mut self, descriptor: &DvcPipeProxyDescriptor) {
            self.descriptors.push(descriptor.clone());
        }

        pub fn get_message_sink(&self) -> Box<DvcPipeProxyMessageSink> {
            Box::new(self.message_sink.clone())
        }
    }
}
