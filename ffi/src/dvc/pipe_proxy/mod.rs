use ironrdp::svc::SvcMessage;

const IO_CHANNEL_SIZE: usize = 100;

#[diplomat::bridge]
pub mod ffi {
    use crate::error::ffi::IronRdpError;
    use ironrdp::pdu::{pdu_other_err, PduError};
    use std::sync::mpsc;

    #[diplomat::opaque]
    pub struct SendDvcChannelDataMessage(pub super::SendDvcChannelDataMessageInner);

    #[diplomat::opaque]
    pub struct DvcPipeProxyChannelsList {
        channels: Vec<(String, String)>,
    }

    impl DvcPipeProxyChannelsList {
        pub fn new() -> Box<DvcPipeProxyChannelsList> {
            DvcPipeProxyChannelsList { channels: Vec::new() }.into()
        }

        pub fn add_channel(&mut self, channel_name: String, pipe_name: String) {
            self.channels.push((channel_name, pipe_name));
        }
    }

    #[diplomat::opaque]
    pub struct DvcPipeProxyFactory {
        inner: super::DvcPipeProxyFactoryInner,
        tx: mpsc::SyncSender<super::SendDvcChannelDataMessageInner>,
    }

    impl DvcPipeProxyFactory {
        pub fn build_proxies(&self) -> Result<Box<DvcNamedPipeProxyChannels>, Box<IronRdpError>> {
            let mut proxies = Vec::new();
            for (channel_name, pipe_name) in &self.inner.channels {
                let tx = self.tx.clone();

                let proxy = ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy::new(
                    channel_name,
                    pipe_name,
                    move |channel_id, svc_messages| {
                        let message = super::SendDvcChannelDataMessageInner {
                            channel_id,
                            svc_messages,
                        };

                        tx.send(message)
                            .map_err(|_| pdu_other_err!("Failed to send DVC channel data message"))
                    },
                );
                proxies.push(super::DvcNamedPipeProxyInner { proxy });
            }
            Ok(DvcNamedPipeProxyChannels(proxies).into())
        }
    }

    #[diplomat::opaque]
    pub struct DvcPipeProxyManager {
        tx: mpsc::SyncSender<super::SendDvcChannelDataMessageInner>,
        rx: mpsc::Receiver<super::SendDvcChannelDataMessageInner>,
        channels: Vec<(String, String)>,
    }

    impl DvcPipeProxyManager {
        pub fn new(channels_set: &DvcPipeProxyChannelsList) -> Box<DvcPipeProxyManager> {
            let (tx, rx) = mpsc::sync_channel::<super::SendDvcChannelDataMessageInner>(super::IO_CHANNEL_SIZE);

            let channels = channels_set.channels.clone();

            DvcPipeProxyManager { tx, rx, channels }.into()
        }

        pub fn build_pipe_proxy_factory(&self) -> Box<DvcPipeProxyFactory> {
            let inner = super::DvcPipeProxyFactoryInner {
                channels: self.channels.clone(),
            };

            DvcPipeProxyFactory {
                inner,
                tx: self.tx.clone(),
            }
            .into()
        }

        fn next_clipboard_message(&self) -> Result<Option<SendDvcChannelDataMessage>, Box<IronRdpError>> {
            Ok(self.rx.try_recv().ok().map(SendDvcChannelDataMessage))
        }

        fn next_clipboard_message_blocking(
            &self,
        ) -> Result<SendDvcChannelDataMessage, Box<IronRdpError>> {
            Ok(self
                .rx
                .recv()
                .map(SendDvcChannelDataMessage)
                .map_err(|_| "Failed to receive clipboard message")?
            )
        }
    }

    #[diplomat::opaque]
    pub struct DvcNamedPipeProxyChannels(pub Vec<super::DvcNamedPipeProxyInner>);
}

pub(crate) struct SendDvcChannelDataMessageInner {
    channel_id: u32,
    svc_messages: Vec<SvcMessage>,
}

impl SendDvcChannelDataMessageInner {
    pub(crate) fn take_messages(&mut self) -> Vec<SvcMessage> {
        core::mem::take(&mut self.svc_messages)
    }
}

pub(crate) struct DvcPipeProxyFactoryInner {
    channels: Vec<(String, String)>,
}

impl DvcPipeProxyFactoryInner {
    pub(crate) fn new(channels: Vec<(String, String)>) -> Self {
        Self { channels }
    }

    pub(crate) fn channels(&self) -> impl Iterator<Item = (&str, &str)> {
        self.channels
            .iter()
            .map(|(channel_name, pipe_name)| (channel_name.as_str(), pipe_name.as_str()))
    }
}

pub(crate) struct DvcNamedPipeProxyInner {
    pub(crate) proxy: ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy,
}
