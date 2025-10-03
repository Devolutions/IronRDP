use std::sync::mpsc;

use ironrdp::svc::SvcMessage;

#[diplomat::bridge]
pub mod ffi {
    use std::sync::mpsc;

    use super::{DvcPipeProxyMessageInner, DvcPipeProxyMessageQueueInner};
    use crate::error::ffi::IronRdpError;

    #[diplomat::opaque]
    pub struct DvcPipeProxyMessage(pub DvcPipeProxyMessageInner);

    impl DvcPipeProxyMessage {
        pub fn get_channel_id(&self) -> u32 {
            self.0 .0
        }
    }

    #[diplomat::opaque]
    #[derive(Clone)]
    pub struct DvcPipeProxyMessageSink(pub mpsc::SyncSender<DvcPipeProxyMessageInner>);

    #[diplomat::opaque]
    pub struct DvcPipeProxyMessageQueue(DvcPipeProxyMessageQueueInner);

    impl DvcPipeProxyMessageQueue {
        pub fn new(queue_size: u32) -> Box<DvcPipeProxyMessageQueue> {
            #[expect(clippy::missing_panics_doc, reason = "unreachable panic (integer upcast)")]
            let queue_size = usize::try_from(queue_size).expect("invalid dvc pipe proxy message queue size");

            Box::new(DvcPipeProxyMessageQueue(DvcPipeProxyMessageQueueInner::new(queue_size)))
        }

        pub fn next_message(&self) -> Result<Option<Box<DvcPipeProxyMessage>>, Box<IronRdpError>> {
            Ok(self.0.next_message().map(DvcPipeProxyMessage).map(Box::new))
        }

        pub fn next_message_blocking(&self) -> Result<Box<DvcPipeProxyMessage>, Box<IronRdpError>> {
            let message = self.0.next_message_blocking().map(DvcPipeProxyMessage).map(Box::new)?;

            Ok(message)
        }

        pub fn get_sink(&self) -> Box<DvcPipeProxyMessageSink> {
            Box::new(DvcPipeProxyMessageSink(self.0.tx.clone()))
        }
    }
}

struct DvcPipeProxyMessageQueueInner {
    tx: mpsc::SyncSender<DvcPipeProxyMessageInner>,
    rx: mpsc::Receiver<DvcPipeProxyMessageInner>,
}

impl DvcPipeProxyMessageQueueInner {
    fn new(queue_size: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel(queue_size);
        Self { tx, rx }
    }

    fn next_message(&self) -> Option<DvcPipeProxyMessageInner> {
        self.rx.try_recv().ok()
    }

    fn next_message_blocking(&self) -> Result<DvcPipeProxyMessageInner, &'static str> {
        self.rx.recv().map_err(|_| "failed to receive dvc pipe proxy message")
    }
}

pub struct DvcPipeProxyMessageInner(pub u32, pub Vec<SvcMessage>);
