pub mod activation;
pub mod config;
pub mod result;
pub mod state;

#[diplomat::bridge]
pub mod ffi {
    use core::fmt::Write as _;

    use diplomat_runtime::DiplomatWriteable;
    use ironrdp::connector::Sequence as _;
    use ironrdp::displaycontrol::client::DisplayControlClient;
    use ironrdp::dvc::DvcProcessor;
    use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
    use tracing::info;

    use super::config::ffi::Config;
    use super::result::ffi::Written;
    use super::state::ffi::ClientConnectorState;
    use crate::clipboard::ffi::Cliprdr;
    use crate::dvc::dvc_pipe_proxy_message_queue::DvcPipeProxyMessageInner;
    use crate::dvc::ffi::DvcPipeProxyConfig;
    use crate::error::ffi::{IronRdpError, IronRdpErrorKind};
    use crate::error::ValueConsumedError;
    use crate::pdu::ffi::WriteBuf;

    #[diplomat::opaque] // We must use Option here, as ClientConnector is not Clone and have functions that consume it
    pub struct ClientConnector(pub Option<ironrdp::connector::ClientConnector>);

    // Basic Impl for ClientConnector
    impl ClientConnector {
        pub fn new(config: &Config, client_addr: &str) -> Result<Box<ClientConnector>, Box<IronRdpError>> {
            let client_addr = client_addr.parse().map_err(|_| IronRdpErrorKind::Generic)?;

            Ok(Box::new(ClientConnector(Some(
                ironrdp::connector::ClientConnector::new(config.connector.clone(), client_addr),
            ))))
        }

        // FIXME: Naming: since this is not a builder pattern, use "attach"?
        // FIXME: We need to create opaque for ironrdp::svc::StaticChannelSet
        /// Must use
        pub fn with_static_channel_rdp_snd(&mut self) -> Result<(), Box<IronRdpError>> {
            use ironrdp::rdpsnd::client::{NoopRdpsndBackend, Rdpsnd};

            let Some(connector) = self.0.take() else {
                return Err(IronRdpErrorKind::Consumed.into());
            };

            self.0 = Some(connector.with_static_channel(Rdpsnd::new(Box::new(NoopRdpsndBackend {}))));

            Ok(())
        }

        // FIXME: We need to create opaque for ironrdp::rdpdr::Rdpdr
        /// Must use
        pub fn with_static_channel_rdpdr(
            &mut self,
            computer_name: &str,
            smart_card_device_id: u32,
        ) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.take() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            self.0 = Some(
                connector.with_static_channel(
                    ironrdp::rdpdr::Rdpdr::new(Box::new(ironrdp::rdpdr::NoopRdpdrBackend {}), computer_name.to_owned())
                        .with_smartcard(smart_card_device_id),
                ),
            );

            Ok(())
        }

        fn with_dvc<T>(&mut self, processor: T) -> Result<(), Box<IronRdpError>>
        where
            T: DvcProcessor + 'static,
        {
            let Some(connector) = &mut self.0 else {
                return Err(ValueConsumedError::for_item("connector").into());
            };

            let drdynvc = match connector.get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>() {
                Some(processor) => processor,
                None => {
                    connector.attach_static_channel(ironrdp::dvc::DrdynvcClient::new());
                    connector
                        .get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
                        .expect("DrdynvcClient should be initialized above")
                }
            };

            drdynvc.attach_dynamic_channel(processor);

            Ok(())
        }

        pub fn with_dynamic_channel_display_control(&mut self) -> Result<(), Box<IronRdpError>> {
            self.with_dvc(DisplayControlClient::new(|c| {
                info!(DisplayCountrolCapabilities = ?c, "DisplayControl capabilities received");
                Ok(Vec::new())
            }))
        }

        pub fn with_dynamic_channel_pipe_proxy(
            &mut self,
            config: &DvcPipeProxyConfig,
        ) -> Result<(), Box<IronRdpError>> {
            for descriptor in &config.descriptors {
                let sink = config.message_sink.0.clone();
                let proxy = DvcNamedPipeProxy::new(
                    &descriptor.channel_name,
                    &descriptor.pipe_name,
                    move |channel_id, svc_message| {
                        let _ = sink.send(DvcPipeProxyMessageInner(channel_id, svc_message));
                        Ok(())
                    },
                );
                self.with_dvc(proxy)?;
            }
            Ok(())
        }

        pub fn should_perform_security_upgrade(&self) -> Result<bool, Box<IronRdpError>> {
            let Some(connector) = self.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            Ok(connector.should_perform_security_upgrade())
        }

        pub fn mark_security_upgrade_as_done(&mut self) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.as_mut() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            connector.mark_security_upgrade_as_done();
            Ok(())
        }

        pub fn should_perform_credssp(&self) -> Result<bool, Box<IronRdpError>> {
            let Some(connector) = self.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };

            Ok(connector.should_perform_credssp())
        }

        pub fn mark_credssp_as_done(&mut self) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.as_mut() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            connector.mark_credssp_as_done();
            Ok(())
        }

        pub fn step(&mut self, input: &[u8], write_buf: &mut WriteBuf) -> Result<Box<Written>, Box<IronRdpError>> {
            let Some(connector) = self.0.as_mut() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            let written = connector.step(input, &mut write_buf.0)?;
            Ok(Box::new(Written(written)))
        }

        pub fn step_no_input(&mut self, write_buf: &mut WriteBuf) -> Result<Box<Written>, Box<IronRdpError>> {
            let Some(connector) = self.0.as_mut() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            let written = connector.step_no_input(&mut write_buf.0)?;
            Ok(Box::new(Written(written)))
        }

        pub fn attach_static_cliprdr(&mut self, cliprdr: &mut Cliprdr) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.as_mut() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };

            let Some(cliprdr) = cliprdr.0.take() else {
                return Err(ValueConsumedError::for_item("cliprdr").into());
            };

            connector.attach_static_channel(cliprdr);
            Ok(())
        }
    }

    #[diplomat::opaque]
    pub struct PduHint<'a>(pub &'a dyn ironrdp::pdu::PduHint);

    impl<'a> PduHint<'a> {
        pub fn find_size(&'a self, bytes: &[u8]) -> Result<Box<crate::utils::ffi::OptionalUsize>, Box<IronRdpError>> {
            let pdu_hint = self.0;
            // TODO C# NuGet is only used on client-side so we probably don’t need to break the ABI for that just now.
            let size = pdu_hint.find_size(bytes)?.map(|(_match, size)| size);
            Ok(Box::new(crate::utils::ffi::OptionalUsize(size)))
        }
    }

    #[diplomat::opaque]
    pub struct DynState<'a>(pub &'a dyn ironrdp::connector::State);

    impl<'a> DynState<'a> {
        pub fn get_name(&'a self, writeable: &'a mut DiplomatWriteable) -> Result<(), Box<IronRdpError>> {
            let name = self.0.name();
            write!(writeable, "{name}")?;
            Ok(())
        }

        pub fn is_terminal(&'a self) -> bool {
            self.0.is_terminal()
        }
    }

    impl ClientConnector {
        pub fn next_pdu_hint(&self) -> Result<Option<Box<PduHint<'_>>>, Box<IronRdpError>> {
            let Some(connector) = self.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            tracing::trace!(pduhint=?connector.next_pdu_hint(), "Reading next PDU hint");
            Ok(connector.next_pdu_hint().map(PduHint).map(Box::new))
        }

        pub fn get_dyn_state(&self) -> Result<Box<DynState<'_>>, Box<IronRdpError>> {
            let Some(connector) = self.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            Ok(Box::new(DynState(connector.state())))
        }

        pub fn consume_and_cast_to_client_connector_state(
            &mut self,
        ) -> Result<Box<ClientConnectorState>, Box<IronRdpError>> {
            let Some(connector) = self.0.take() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            Ok(Box::new(ClientConnectorState(Some(connector.state))))
        }
    }

    #[diplomat::opaque]
    pub struct ChannelConnectionSequence(pub ironrdp::connector::ChannelConnectionSequence);

    #[diplomat::opaque]
    pub struct LicenseExchangeSequence(pub ironrdp::connector::LicenseExchangeSequence);
}
