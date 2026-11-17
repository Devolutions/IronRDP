pub mod activation;
pub mod config;
pub mod result;
pub mod state;

#[diplomat::bridge]
pub mod ffi {
    use core::fmt::Write as _;

    use diplomat_runtime::DiplomatWrite;
    use ironrdp::connector::Sequence as _;
    use ironrdp::displaycontrol::client::DisplayControlClient;
    use ironrdp::dvc::DvcClientProcessor;
    use ironrdp::rdpei::RdpeiClient;
    use ironrdp::rdpei::pdu::{CsReadyFlags, RdpInputProtocolVersion};
    use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
    use tracing::info;

    use super::config::ffi::Config;
    use super::result::ffi::Written;
    use super::state::ffi::ClientConnectorState;
    use crate::audio::ffi::NativeAudio;
    use crate::clipboard::ffi::Cliprdr;
    use crate::dvc::dvc_pipe_proxy_message_queue::DvcPipeProxyMessageInner;
    use crate::dvc::ffi::DvcPipeProxyConfig;
    use crate::error::ValueConsumedError;
    use crate::error::ffi::{IronRdpError, IronRdpErrorKind};
    use crate::pdu::ffi::WriteBuf;
    use crate::utils::monotonic_now;

    #[diplomat::opaque_mut] // We must use Option here, as ClientConnector is not Clone and have functions that consume it
    pub struct ClientConnector(
        pub(crate) Option<ironrdp::connector::ClientConnector>,
        pub(crate) Option<std::sync::Arc<crate::audio::AudioShared>>,
    );

    // Basic Impl for ClientConnector
    impl ClientConnector {
        pub fn new(config: &Config, client_addr: &str) -> Result<Box<ClientConnector>, Box<IronRdpError>> {
            let client_addr = client_addr.parse().map_err(|_| IronRdpErrorKind::Generic)?;

            Ok(Box::new(ClientConnector(
                Some(ironrdp::connector::ClientConnector::new(
                    config.connector.clone(),
                    client_addr,
                )),
                None,
            )))
        }

        // FIXME: Naming: since this is not a builder pattern, use "attach"?
        // FIXME: We need to create opaque for ironrdp::svc::StaticChannelSet
        /// Must use
        pub fn with_static_channel_rdp_snd(&mut self) -> Result<(), Box<IronRdpError>> {
            use ironrdp::rdpsnd::client::{NoopRdpsndBackend, Rdpsnd};

            if self.1.as_ref().is_some_and(|audio| audio.has_playback()) {
                return Err("native playback is already attached".into());
            }
            let Some(connector) = self.0.take() else {
                return Err(IronRdpErrorKind::Consumed.into());
            };

            let mut connector = connector.with_static_channel(Rdpsnd::new(Box::new(NoopRdpsndBackend {})));
            connector.config.enable_audio_playback = true;
            self.0 = Some(connector);

            Ok(())
        }

        /// Attaches native audio to this connector.
        ///
        /// Use a fresh `NativeAudio` for each connection.
        /// Playback attaches RDPDR without redirected devices when the channel is absent.
        /// Existing RDPDR configuration is preserved.
        pub fn with_native_audio(&mut self, audio: &NativeAudio) -> Result<(), Box<IronRdpError>> {
            let connector = self
                .0
                .as_mut()
                .ok_or_else(|| ValueConsumedError::for_item("connector"))?;
            if self.1.is_some() {
                return Err("native audio is already attached to this connector".into());
            }
            if !matches!(
                connector.state,
                ironrdp::connector::ClientConnectorState::ConnectionInitiationSendRequest
            ) {
                return Err("native audio must be attached before connecting".into());
            }
            audio.0.attach(connector)?;
            if let Some(drdynvc) = connector.get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>() {
                let playback = drdynvc.has_channel_name(ironrdp::rdpsnd::client::DVC_CHANNEL_NAME)
                    || drdynvc.has_channel_name(ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME);
                let capture = drdynvc.has_channel_name(ironrdp_rdpeai::CHANNEL_NAME);
                connector.config.enable_audio_playback |= playback;
                connector.config.enable_audio_capture |= capture;
            }
            self.1 = Some(std::sync::Arc::clone(&audio.0));
            Ok(())
        }

        // FIXME: We need to create opaque for ironrdp::rdpdr::Rdpdr
        /// Must use
        ///
        /// Announces a smartcard device. On Windows, calls are passed through to the local
        /// WinSCard service; elsewhere every call fails with `SCARD_E_NO_SERVICE`.
        ///
        /// Some WinSCard calls complete asynchronously: call `ActiveStage::poll_rdpdr_deferred`
        /// periodically to send their results to the server.
        pub fn with_static_channel_rdpdr(
            &mut self,
            computer_name: &str,
            smart_card_device_id: u32,
        ) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.take() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };

            #[cfg(windows)]
            let backend: Box<dyn ironrdp::rdpdr::RdpdrBackend> = Box::new(
                ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::from_drives(Vec::new())
                    .map_err(|error| {
                        crate::error::GenericError(
                            anyhow::Error::new(error).context("invalid RDPDR backend configuration"),
                        )
                    })?
                    .with_smartcard(true)
                    .build(),
            );
            #[cfg(not(windows))]
            let backend: Box<dyn ironrdp::rdpdr::RdpdrBackend> = Box::new(ironrdp::rdpdr::NoopRdpdrBackend);

            self.0 = Some(connector.with_static_channel(
                ironrdp::rdpdr::Rdpdr::new(backend, computer_name.to_owned()).with_smartcard(smart_card_device_id),
            ));

            Ok(())
        }

        fn with_dvc<T>(&mut self, processor: T) -> Result<(), Box<IronRdpError>>
        where
            T: DvcClientProcessor + 'static,
        {
            if self
                .1
                .as_ref()
                .is_some_and(|audio| audio.reserves_dynamic_channel(processor.channel_name()))
            {
                return Err("native audio dynamic channel is reserved".into());
            }
            let Some(connector) = &mut self.0 else {
                return Err(ValueConsumedError::for_item("connector").into());
            };

            let playback = matches!(
                processor.channel_name(),
                ironrdp::rdpsnd::client::DVC_CHANNEL_NAME | ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME
            );
            let capture = processor.channel_name() == ironrdp_rdpeai::CHANNEL_NAME;
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
            connector.config.enable_audio_playback |= playback;
            connector.config.enable_audio_capture |= capture;

            Ok(())
        }

        pub fn with_dynamic_channel_display_control(&mut self) -> Result<(), Box<IronRdpError>> {
            self.with_dvc(DisplayControlClient::new(|c| {
                info!(display_control_capabilities = ?c, "DisplayControl capabilities received");
                Ok(Vec::new())
            }))
        }

        /// Attaches the MS-RDPEI dynamic channel so the client can send touch frames.
        ///
        /// `max_touch_contacts` is the `maxTouchContacts` advertised in `RDPINPUT_CS_READY_PDU`
        /// ([MS-RDPEI] 2.2.3.2 "Client Ready PDU") and is the limit to give `TouchFrameBuilder`.
        /// `show_touch_visuals` asks the server to draw its touch visuals.
        /// Protocol version 2 is advertised, which covers touch and pen frames; multipen injection is
        /// not requested, so pen frames use `deviceId` 0.
        ///
        /// [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d
        pub fn with_dynamic_channel_rdpei(
            &mut self,
            max_touch_contacts: u16,
            show_touch_visuals: bool,
        ) -> Result<(), Box<IronRdpError>> {
            let flags = if show_touch_visuals {
                CsReadyFlags::SHOW_TOUCH_VISUALS
            } else {
                CsReadyFlags::empty()
            };
            self.with_dvc(RdpeiClient::new(
                max_touch_contacts,
                flags,
                RdpInputProtocolVersion::V200,
            ))
        }

        pub fn with_dynamic_channel_pipe_proxy(
            &mut self,
            config: &DvcPipeProxyConfig,
        ) -> Result<(), Box<IronRdpError>> {
            if self.1.as_ref().is_some_and(|audio| {
                config
                    .descriptors
                    .iter()
                    .any(|descriptor| audio.reserves_dynamic_channel(&descriptor.channel_name))
            }) {
                return Err("native audio dynamic channel is reserved".into());
            }

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

        /// Send X.224 with an explicit protocol set (VMConnect uses HYBRID only).
        pub fn initiate_with_security_protocol(
            &mut self,
            security_protocol: u32,
            write_buf: &mut WriteBuf,
        ) -> Result<Box<Written>, Box<IronRdpError>> {
            let Some(connector) = self.0.as_mut() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            let security_protocol = ironrdp::pdu::nego::SecurityProtocol::from_bits(security_protocol)
                .ok_or_else(|| ironrdp::connector::general_err!("invalid security protocol"))?;
            let written = connector.initiate_with_security_protocol(security_protocol, &mut write_buf.0)?;
            Ok(Box::new(Written(written)))
        }

        /// Drop host identity after pre-X.224 CredSSP so it is not forwarded into guest RDP.
        pub fn clear_credentials_after_host_auth(&mut self) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.as_mut() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            connector.config.credentials = ironrdp::connector::Credentials::UsernamePassword {
                username: String::new(),
                password: String::new(),
            };
            connector.config.domain = None;
            connector.config.autologon = false;
            Ok(())
        }

        /// Require EnhancedSecurityUpgrade with HYBRID, matching ironrdp-vmconnect::connect_front.
        pub fn ensure_selected_hybrid(&self) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            ironrdp_vmconnect::ensure_selected_credssp(&connector.state).map_err(Into::into)
        }

        pub fn step(&mut self, input: &[u8], write_buf: &mut WriteBuf) -> Result<Box<Written>, Box<IronRdpError>> {
            let Some(connector) = self.0.as_mut() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            // The FFI surface has no parameter through which the .NET caller could pass
            // when it read `input`, so this stamps on entry to the call instead. That is
            // at least as accurate as the other drivers' `last_read_at`, which is also a
            // post-read stamp taken right after the read completes rather than exactly
            // when the first byte arrived.
            let written = connector.step(input, Some(monotonic_now()), &mut write_buf.0)?;
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

        pub fn next_pdu_hint<'a>(&'a self) -> Result<Option<Box<PduHint<'a>>>, Box<IronRdpError>> {
            let Some(connector) = self.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            tracing::trace!(pdu_hint=?connector.next_pdu_hint(), "Reading next PDU hint");
            Ok(connector.next_pdu_hint().map(PduHint).map(Box::new))
        }

        #[diplomat::attr(auto, getter = "dyn_state")]
        pub fn get_dyn_state<'a>(&'a self) -> Result<Box<DynState<'a>>, Box<IronRdpError>> {
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
            Ok(Box::new(ClientConnectorState(Some(connector.state), self.1.take())))
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
        #[diplomat::attr(auto, getter = "name")]
        pub fn get_name(&'a self, writeable: &'a mut DiplomatWrite) -> Result<(), Box<IronRdpError>> {
            let name = self.0.name();
            write!(writeable, "{name}")?;
            Ok(())
        }

        pub fn is_terminal(&'a self) -> bool {
            self.0.is_terminal()
        }
    }

    #[diplomat::opaque]
    pub struct ChannelConnectionSequence(pub ironrdp::connector::ChannelConnectionSequence);

    #[diplomat::opaque]
    pub struct LicenseExchangeSequence(pub ironrdp::connector::LicenseExchangeSequence);
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::ffi::ClientConnector;
    use crate::audio::ffi::NativeAudio;
    use crate::connector::config::ffi::{ConfigBuilder, PerformanceFlags};
    use crate::dvc::dvc_pipe_proxy_message_queue::ffi::DvcPipeProxyMessageQueue;
    use crate::dvc::ffi::{DvcPipeProxyConfig, DvcPipeProxyDescriptor};

    fn connector_without_playback() -> Result<ClientConnector, &'static str> {
        let mut builder = ConfigBuilder::new();
        builder.with_username_and_password("user", "password");
        builder.set_desktop_size(768, 1024);
        builder.set_client_name("client");
        builder.set_client_dir("C:\\");
        builder.set_performance_flags(&PerformanceFlags::new_default());
        builder.no_audio_playback = Some(true);
        let config = builder.build().map_err(|_| "connector config")?;
        let client_addr = "127.0.0.1:0".parse().map_err(|_| "test address")?;

        Ok(ClientConnector(
            Some(ironrdp::connector::ClientConnector::new(config.connector, client_addr)),
            None,
        ))
    }

    fn capture_audio() -> Result<Box<NativeAudio>, &'static str> {
        NativeAudio::new(false, true, "", "", 0).map_err(|_| "audio settings")
    }

    fn attach_pipe_proxy(connector: &mut ClientConnector, channel_name: &str) -> bool {
        let queue = DvcPipeProxyMessageQueue::new(1);
        let sink = queue.get_sink();
        let descriptor = DvcPipeProxyDescriptor::new(channel_name, "test-pipe");
        let mut config = DvcPipeProxyConfig::new(&sink);
        config.add_pipe_proxy(&descriptor);

        connector.with_dynamic_channel_pipe_proxy(&config).is_ok()
    }

    // oracle: regression #2469
    #[test]
    fn playback_proxy_attached_after_capture_advertises_playback() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;
        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        ));

        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn playback_proxy_attached_before_capture_advertises_playback() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        ));
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;

        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn lossy_playback_proxy_before_capture_advertises_playback() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME
        ));
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;

        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn lossy_playback_proxy_after_capture_advertises_playback() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;
        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME
        ));

        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn input_proxy_attached_before_playback_advertises_capture() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        assert!(attach_pipe_proxy(&mut connector, ironrdp_rdpeai::CHANNEL_NAME));
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_capture);
        let playback = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio settings")?;
        connector
            .with_native_audio(&playback)
            .map_err(|_| "attach playback-only audio")?;

        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_capture);
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn input_proxy_attached_after_playback_advertises_capture() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let playback = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio settings")?;
        connector
            .with_native_audio(&playback)
            .map_err(|_| "attach playback-only audio")?;
        assert!(attach_pipe_proxy(&mut connector, ironrdp_rdpeai::CHANNEL_NAME));

        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_capture);
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn unrelated_proxy_does_not_advertise_audio() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        assert!(attach_pipe_proxy(&mut connector, "unrelated"));

        let config = &connector.0.as_ref().ok_or("connector")?.config;
        assert!(!config.enable_audio_playback);
        assert!(!config.enable_audio_capture);
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn separately_attached_playback_survives_later_capture_configuration() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        connector
            .with_static_channel_rdp_snd()
            .map_err(|_| "attach playback channel")?;
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;

        assert!(
            connector
                .0
                .as_ref()
                .is_some_and(|connector| connector.config.enable_audio_playback)
        );
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn separately_attached_playback_after_capture_is_advertised() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;
        assert!(
            !connector
                .0
                .as_ref()
                .is_some_and(|connector| connector.config.enable_audio_playback)
        );

        connector
            .with_static_channel_rdp_snd()
            .map_err(|_| "attach playback channel")?;

        assert!(
            connector
                .0
                .as_ref()
                .is_some_and(|connector| connector.config.enable_audio_playback)
        );
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn capture_only_audio_allows_playback_pipe_proxy() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;

        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        ));
        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME
        ));
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn capture_only_audio_reserves_input_pipe_proxy() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;

        assert!(!attach_pipe_proxy(&mut connector, ironrdp_rdpeai::CHANNEL_NAME));
        Ok(())
    }

    // oracle: regression Devolutions/IronRDP#2469, review comment 4604743100
    #[test]
    fn reserved_pipe_proxy_rejects_all_descriptors_without_mutation() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;
        let queue = DvcPipeProxyMessageQueue::new(1);
        let mut config = DvcPipeProxyConfig::new(&queue.get_sink());
        config.add_pipe_proxy(&DvcPipeProxyDescriptor::new(
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME,
            "playback-pipe",
        ));
        config.add_pipe_proxy(&DvcPipeProxyDescriptor::new(ironrdp_rdpeai::CHANNEL_NAME, "input-pipe"));

        assert!(connector.with_dynamic_channel_pipe_proxy(&config).is_err());
        let inner = connector.0.as_mut().ok_or("connector")?;
        assert!(!inner.config.enable_audio_playback);
        let drdynvc = inner
            .get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
            .ok_or("dynamic channels")?;
        assert!(!drdynvc.has_channel_name(ironrdp::rdpsnd::client::DVC_CHANNEL_NAME));
        assert!(drdynvc.has_channel_name(ironrdp_rdpeai::CHANNEL_NAME));
        Ok(())
    }

    // oracle: regression Devolutions/IronRDP#2469, review comment 4604743100
    #[test]
    fn rejected_proxy_batch_preserves_proxy_attached_before_audio() -> Result<(), &'static str> {
        use std::sync::mpsc::{self, TryRecvError};

        use crate::dvc::dvc_pipe_proxy_message_queue::ffi::DvcPipeProxyMessageSink;

        let mut connector = connector_without_playback()?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let sink = DvcPipeProxyMessageSink(sender);
        let mut original = DvcPipeProxyConfig::new(&sink);
        original.add_pipe_proxy(&DvcPipeProxyDescriptor::new("other", "original-pipe"));
        connector
            .with_dynamic_channel_pipe_proxy(&original)
            .map_err(|_| "attach original proxy")?;
        drop(original);
        drop(sink);
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));

        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;
        let queue = DvcPipeProxyMessageQueue::new(1);
        let mut replacement = DvcPipeProxyConfig::new(&queue.get_sink());
        replacement.add_pipe_proxy(&DvcPipeProxyDescriptor::new("other", "replacement-pipe"));
        replacement.add_pipe_proxy(&DvcPipeProxyDescriptor::new(ironrdp_rdpeai::CHANNEL_NAME, "input-pipe"));

        assert!(connector.with_dynamic_channel_pipe_proxy(&replacement).is_err());
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        Ok(())
    }

    // oracle: regression Devolutions/IronRDP#2469, review comment 4604743100
    #[test]
    fn valid_proxy_batch_attaches_all_descriptors_after_audio() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;
        let queue = DvcPipeProxyMessageQueue::new(1);
        let mut config = DvcPipeProxyConfig::new(&queue.get_sink());
        config.add_pipe_proxy(&DvcPipeProxyDescriptor::new("first", "first-pipe"));
        config.add_pipe_proxy(&DvcPipeProxyDescriptor::new("second", "second-pipe"));

        connector
            .with_dynamic_channel_pipe_proxy(&config)
            .map_err(|_| "attach proxies")?;
        let drdynvc = connector
            .0
            .as_mut()
            .ok_or("connector")?
            .get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
            .ok_or("dynamic channels")?;
        assert!(drdynvc.has_channel_name("first"));
        assert!(drdynvc.has_channel_name("second"));
        assert!(drdynvc.has_channel_name(ironrdp_rdpeai::CHANNEL_NAME));
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn playback_proxy_attached_before_capture_only_audio_is_preserved() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        ));

        let capture = capture_audio()?;
        connector
            .with_native_audio(&capture)
            .map_err(|_| "attach capture-only audio")?;
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn playback_only_audio_allows_input_pipe_proxy() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let playback = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio settings")?;
        connector
            .with_native_audio(&playback)
            .map_err(|_| "attach playback-only audio")?;

        assert!(attach_pipe_proxy(&mut connector, ironrdp_rdpeai::CHANNEL_NAME));
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn playback_only_audio_reserves_playback_pipe_proxy() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let playback = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio settings")?;
        connector
            .with_native_audio(&playback)
            .map_err(|_| "attach playback-only audio")?;

        assert!(!attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        ));
        assert!(!attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::LOSSY_DVC_CHANNEL_NAME
        ));
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn input_proxy_attached_before_capture_audio_is_rejected() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        assert!(attach_pipe_proxy(&mut connector, ironrdp_rdpeai::CHANNEL_NAME));

        let capture = capture_audio()?;
        assert!(connector.with_native_audio(&capture).is_err());
        Ok(())
    }

    // oracle: regression #2469
    #[test]
    fn playback_proxy_attached_before_native_playback_is_rejected() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        ));
        let playback = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio settings")?;

        assert!(connector.with_native_audio(&playback).is_err());
        Ok(())
    }
}
