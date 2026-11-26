pub mod activation;
pub mod config;
pub mod result;
pub mod state;

#[derive(Default)]
pub(crate) struct RdpdrConfiguration {
    drives: Vec<config::RedirectedDriveConfig>,
    smartcard: bool,
    audio_playback_forced: bool,
    /// INVARIANT: This is true only while the automatically installed silent RDPSND channel is present.
    compatibility_rdpsnd: bool,
}

fn attach_rdpdr_compatibility_rdpsnd(connector: &mut ironrdp::connector::ClientConnector, audio_playback_forced: bool) {
    let playback_dvc = connector
        .get_static_channel_processor::<ironrdp::dvc::DrdynvcClient>()
        .is_some_and(|drdynvc| crate::audio::has_audio_channel(drdynvc, crate::audio::AudioChannelKind::Playback));
    connector.config.enable_audio_playback = audio_playback_forced || playback_dvc;
    connector.attach_static_channel(ironrdp::rdpsnd::client::Rdpsnd::new(Box::new(
        ironrdp::rdpsnd::client::NoopRdpsndBackend,
    )));
}

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
    use crate::audio::{AudioChannelKind, audio_channel_kind};
    use crate::clipboard::ffi::Cliprdr;
    use crate::dvc::direct_channel::DirectDvcChannelProcessor;
    use crate::dvc::direct_channel::ffi::DirectDvcConfig;
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
        pub(crate) super::RdpdrConfiguration,
    );

    // Basic Impl for ClientConnector
    impl ClientConnector {
        fn has_direct_dvc_listener(drdynvc: &ironrdp::dvc::DrdynvcClient, channel_name: &str) -> bool {
            drdynvc.has_channel_name_of_type::<DirectDvcChannelProcessor>(channel_name)
        }

        fn create_rdpdr(
            &self,
            computer_name: &str,
            smart_card_device_id: Option<u32>,
        ) -> Result<ironrdp::rdpdr::Rdpdr, Box<IronRdpError>> {
            #[cfg(windows)]
            {
                let drives = self
                    .2
                    .drives
                    .iter()
                    .map(|drive| {
                        ironrdp_rdpdr_native::RedirectedDrive::new(
                            drive.device_id,
                            drive.display_name.clone(),
                            std::path::PathBuf::from(&drive.root_path),
                            drive.read_only,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        crate::error::GenericError(
                            anyhow::Error::new(error).context("invalid redirected drive configuration"),
                        )
                    })?;

                if smart_card_device_id.is_some_and(|id| self.2.drives.iter().any(|drive| drive.device_id == id)) {
                    return Err("smartcard device ID conflicts with a redirected drive".into());
                }

                let factory = ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::from_drives(drives)
                    .map_err(|error| {
                        crate::error::GenericError(
                            anyhow::Error::new(error).context("invalid RDPDR backend configuration"),
                        )
                    })?
                    .with_smartcard(smart_card_device_id.is_some());
                let initial_drives = factory.initial_drives();
                let mut rdpdr = ironrdp::rdpdr::Rdpdr::new(Box::new(factory.build()), computer_name.to_owned());
                if !initial_drives.is_empty() {
                    rdpdr = rdpdr.with_drives(Some(initial_drives));
                }
                if let Some(device_id) = smart_card_device_id {
                    rdpdr = rdpdr.with_smartcard(device_id);
                }
                Ok(rdpdr)
            }
            #[cfg(not(windows))]
            {
                if !self.2.drives.is_empty() {
                    return Err("drive redirection is supported only on Windows".into());
                }
                let backend: Box<dyn ironrdp::rdpdr::RdpdrBackend> = Box::new(ironrdp::rdpdr::NoopRdpdrBackend);
                let mut rdpdr = ironrdp::rdpdr::Rdpdr::new(backend, computer_name.to_owned());
                if let Some(device_id) = smart_card_device_id {
                    rdpdr = rdpdr.with_smartcard(device_id);
                }
                Ok(rdpdr)
            }
        }

        pub fn new(config: &Config, client_addr: &str) -> Result<Box<ClientConnector>, Box<IronRdpError>> {
            let client_addr = client_addr.parse().map_err(|_| IronRdpErrorKind::Generic)?;

            let connector = ironrdp::connector::ClientConnector::new(config.connector.clone(), client_addr);
            let mut client = ClientConnector(
                Some(connector),
                None,
                super::RdpdrConfiguration {
                    drives: config.redirected_drives.clone(),
                    audio_playback_forced: config.no_audio_playback == Some(false),
                    ..Default::default()
                },
            );
            if !client.2.drives.is_empty() {
                // UNTESTED: A drive-only connector must retain each read-only flag through RDPDR I/O,
                // or a read-only share can become writable; check remote writes when changing this path.
                let rdpdr = client.create_rdpdr(&config.connector.client_name, None)?;
                let connector = client
                    .0
                    .as_mut()
                    .ok_or_else(|| ValueConsumedError::for_item("connector"))?;
                connector.attach_static_channel(rdpdr);
                super::attach_rdpdr_compatibility_rdpsnd(connector, client.2.audio_playback_forced);
                client.2.compatibility_rdpsnd = true;
            }

            Ok(Box::new(client))
        }

        // FIXME: Naming: since this is not a builder pattern, use "attach"?
        // FIXME: We need to create opaque for ironrdp::svc::StaticChannelSet
        /// Must use
        #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
        pub fn with_static_channel_rdp_snd(&mut self) -> Result<(), Box<IronRdpError>> {
            use ironrdp::rdpsnd::client::{NoopRdpsndBackend, Rdpsnd};

            if self.1.as_ref().is_some_and(|audio| audio.has_playback()) {
                return Err("native playback is already attached".into());
            }
            let connector = self
                .0
                .as_mut()
                .ok_or_else(|| ValueConsumedError::for_item("connector"))?;
            if !matches!(
                connector.state,
                ironrdp::connector::ClientConnectorState::ConnectionInitiationSendRequest
            ) {
                return Err("RDPSND must be attached before connecting".into());
            }
            if self.2.compatibility_rdpsnd {
                self.2.compatibility_rdpsnd = false;
                connector.config.enable_audio_playback = true;
                return Ok(());
            }
            if connector
                .static_channels
                .get_by_channel_name_key(&Rdpsnd::NAME)
                .is_some()
            {
                return Err("RDPSND channel name is already registered".into());
            }
            if connector.static_channels.len() == ironrdp::svc::MAX_STATIC_CHANNELS {
                return Err("no space for RDPSND channel".into());
            }
            connector.attach_static_channel(Rdpsnd::new(Box::new(NoopRdpsndBackend {})));
            connector.config.enable_audio_playback = true;

            Ok(())
        }

        /// Attaches native audio to this connector.
        ///
        /// Use a fresh `NativeAudio` for each connection.
        /// Playback attaches RDPDR without redirected devices when the channel is absent.
        /// Existing RDPDR configuration is preserved.
        /// An existing RDPSND channel prevents native playback from being attached,
        /// except for the silent compatibility channel installed for RDPDR.
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
            audio.0.attach(connector, self.2.compatibility_rdpsnd)?;
            connector.config.enable_audio_playback |= self.2.audio_playback_forced;
            if audio.0.has_playback() {
                self.2.compatibility_rdpsnd = false;
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
        /// Preserves the drives selected in `ConfigBuilder`.
        /// Drive IDs start at one, so the smartcard device ID must not match a selected drive.
        /// Call before connecting.
        /// When RDPSND is absent, a silent channel is installed because Windows servers do not communicate on RDPDR
        /// without it ([MS-RDPEFS] 2.1, "Transport", Appendix A, note 1).
        ///
        /// Some WinSCard calls complete asynchronously: call `ActiveStage::poll_rdpdr_deferred`
        /// periodically to send their results to the server.
        ///
        /// [MS-RDPEFS]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/fd28bfd9-dae2-4a78-abe1-b4efa208b7aa#Appendix_A_1
        pub fn with_static_channel_rdpdr(
            &mut self,
            computer_name: &str,
            smart_card_device_id: u32,
        ) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            if !matches!(
                connector.state,
                ironrdp::connector::ClientConnectorState::ConnectionInitiationSendRequest
            ) {
                return Err("RDPDR must be attached before connecting".into());
            }
            if self.2.smartcard {
                return Err("smartcard RDPDR is already configured".into());
            }
            let existing_rdpdr = connector
                .static_channels
                .get_by_channel_name_key(&ironrdp::rdpdr::Rdpdr::NAME)
                .is_some();
            if existing_rdpdr
                && connector
                    .static_channels
                    .get_by_type::<ironrdp::rdpdr::Rdpdr>()
                    .is_none()
            {
                return Err("RDPDR channel name is already registered".into());
            }
            let existing_rdpsnd = connector
                .static_channels
                .get_by_channel_name_key(&ironrdp::rdpsnd::client::Rdpsnd::NAME)
                .is_some();
            let needed = usize::from(!existing_rdpdr) + usize::from(!existing_rdpsnd);
            if ironrdp::svc::MAX_STATIC_CHANNELS < connector.static_channels.len() + needed {
                return Err("no space for RDPDR and RDPSND channels".into());
            }

            let rdpdr = self.create_rdpdr(computer_name, Some(smart_card_device_id))?;
            let connector = self
                .0
                .as_mut()
                .ok_or_else(|| ValueConsumedError::for_item("connector"))?;
            connector.attach_static_channel(rdpdr);
            if !existing_rdpsnd {
                super::attach_rdpdr_compatibility_rdpsnd(connector, self.2.audio_playback_forced);
                self.2.compatibility_rdpsnd = true;
            }

            self.2.smartcard = true;

            Ok(())
        }

        #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
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

            let kind = audio_channel_kind(processor.channel_name());
            let playback = kind == Some(AudioChannelKind::Playback);
            let capture = kind == Some(AudioChannelKind::Capture);
            let drdynvc = match connector.get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>() {
                Some(processor) => processor,
                None => {
                    connector.attach_static_channel(ironrdp::dvc::DrdynvcClient::new());
                    connector
                        .get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
                        .expect("DrdynvcClient should be initialized above")
                }
            };

            if Self::has_direct_dvc_listener(drdynvc, processor.channel_name()) {
                return Err("dynamic channel name is already registered as a direct DVC".into());
            }

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

            let has_direct_collision = self.0.as_mut().is_some_and(|connector| {
                connector
                    .get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
                    .is_some_and(|drdynvc| {
                        config
                            .descriptors
                            .iter()
                            .any(|descriptor| Self::has_direct_dvc_listener(drdynvc, &descriptor.channel_name))
                    })
            });
            if has_direct_collision {
                return Err("dynamic channel name is already registered as a direct DVC".into());
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

        /// Registers event-driven listeners for dynamic virtual channels ([MS-RDPEDYC]).
        ///
        /// Rejects duplicate names and names already registered by another listener.
        /// This includes built-in channels and pipe proxies.
        ///
        /// [MS-RDPEDYC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/
        #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
        pub fn with_dynamic_channel_direct(&mut self, config: &DirectDvcConfig) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.as_mut() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };

            let mut channel_names = std::collections::BTreeSet::new();
            for channel_name in &config.channel_names {
                if !channel_names.insert(channel_name) {
                    return Err("direct DVC channel name is registered more than once".into());
                }
            }

            if connector
                .get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
                .is_some_and(|drdynvc| {
                    config
                        .channel_names
                        .iter()
                        .any(|channel_name| drdynvc.has_channel_name(channel_name))
                })
            {
                return Err("dynamic channel name is already registered".into());
            }

            if config.channel_names.is_empty() {
                return Ok(());
            }

            if connector
                .get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
                .is_none()
            {
                connector.attach_static_channel(ironrdp::dvc::DrdynvcClient::new());
            }
            let drdynvc = connector
                .get_static_channel_processor_mut::<ironrdp::dvc::DrdynvcClient>()
                .ok_or("failed to attach DRDYNVC static channel")?;
            for channel_name in &config.channel_names {
                let name = channel_name.clone();
                let sink = config.event_sink.clone();
                drdynvc.attach_dynamic_channel_factory(channel_name.clone(), move || {
                    DirectDvcChannelProcessor::new(&sink, &name)
                });
            }
            for channel_name in &config.channel_names {
                match audio_channel_kind(channel_name) {
                    Some(AudioChannelKind::Playback) => connector.config.enable_audio_playback = true,
                    Some(AudioChannelKind::Capture) => connector.config.enable_audio_capture = true,
                    None => {}
                }
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
    use crate::connector::config::ffi::{AudioPlaybackMode, ConfigBuilder, PerformanceFlags};
    use crate::dvc::direct_channel::ffi::{DirectDvcConfig, DirectDvcEventQueue};
    use crate::dvc::dvc_pipe_proxy_message_queue::ffi::DvcPipeProxyMessageQueue;
    use crate::dvc::ffi::{DvcPipeProxyConfig, DvcPipeProxyDescriptor};

    fn connector_without_playback() -> Result<ClientConnector, &'static str> {
        let mut builder = ConfigBuilder::new();
        builder.with_username_and_password("user", "password");
        builder.set_desktop_size(768, 1024);
        builder.set_client_name("client");
        builder.set_client_dir("C:\\");
        builder.set_performance_flags(&PerformanceFlags::new_default());
        builder.audio_playback_mode = Some(AudioPlaybackMode::Disabled);
        let config = builder.build().map_err(|_| "connector config")?;
        let client_addr = "127.0.0.1:0".parse().map_err(|_| "test address")?;

        Ok(ClientConnector(
            Some(ironrdp::connector::ClientConnector::new(config.connector, client_addr)),
            None,
            Default::default(),
        ))
    }

    fn capture_audio() -> Result<Box<NativeAudio>, &'static str> {
        NativeAudio::new(false, true, "", "", 0).map_err(|_| "audio settings")
    }

    fn default_builder() -> Box<ConfigBuilder> {
        let mut builder = ConfigBuilder::new();
        builder.with_username_and_password("user", "password");
        builder.set_desktop_size(768, 1024);
        builder.set_client_name("client");
        builder.set_client_dir("C:\\");
        builder.set_performance_flags(&PerformanceFlags::new_default());
        builder
    }

    fn default_connector() -> Result<Box<ClientConnector>, &'static str> {
        let builder = default_builder();
        let config = builder.build().map_err(|_| "config")?;
        ClientConnector::new(&config, "127.0.0.1:0").map_err(|_| "connector")
    }

    fn connector_with_playback_setting(setting: Option<bool>) -> Result<Box<ClientConnector>, &'static str> {
        let mut builder = default_builder();
        builder.no_audio_playback = setting;
        let config = builder.build().map_err(|_| "config")?;
        ClientConnector::new(&config, "127.0.0.1:0").map_err(|_| "connector")
    }

    // oracle: mutant enable playback when the caller leaves its setting unspecified
    #[test]
    fn smartcard_compatibility_channel_keeps_default_playback_disabled() -> Result<(), &'static str> {
        let mut connector = connector_with_playback_setting(None)?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert!(!connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: mutant ignore an explicit request to enable audio playback
    #[test]
    fn smartcard_compatibility_channel_honors_explicit_playback_enablement() -> Result<(), &'static str> {
        let mut connector = connector_with_playback_setting(Some(false))?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: mutant ignore an explicit request to disable audio playback
    #[test]
    fn smartcard_compatibility_channel_honors_explicit_playback_disablement() -> Result<(), &'static str> {
        let mut connector = connector_with_playback_setting(Some(true))?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert!(!connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: mutant lose explicit playback enablement when capture is attached before RDPDR
    #[test]
    fn capture_before_smartcard_preserves_explicit_playback() -> Result<(), &'static str> {
        let mut connector = connector_with_playback_setting(Some(false))?;
        connector.with_native_audio(&*capture_audio()?).map_err(|_| "capture")?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: mutant lose explicit playback enablement when capture is attached after RDPDR
    #[test]
    fn capture_after_smartcard_preserves_explicit_playback() -> Result<(), &'static str> {
        let mut connector = connector_with_playback_setting(Some(false))?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        connector.with_native_audio(&*capture_audio()?).map_err(|_| "capture")?;
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    fn rdpsnd_channel_ptr(connector: &ClientConnector) -> Result<*const ironrdp::rdpsnd::client::Rdpsnd, &'static str> {
        connector
            .0
            .as_ref()
            .and_then(|connector| {
                connector
                    .static_channels
                    .get_by_type::<ironrdp::rdpsnd::client::Rdpsnd>()
            })
            .and_then(|channel| channel.channel_processor_downcast_ref::<ironrdp::rdpsnd::client::Rdpsnd>())
            .map(core::ptr::from_ref)
            .ok_or("RDPSND channel")
    }

    /// Checks that playback is advertised through an RDPSND channel other than the silent compatibility one.
    fn assert_rdpsnd_playback(connector: &ClientConnector) -> Result<(), &'static str> {
        rdpsnd_channel_ptr(connector)?;
        assert!(!connector.2.compatibility_rdpsnd);
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    fn assert_native_playback(connector: &ClientConnector) -> Result<(), &'static str> {
        assert!(connector.1.as_ref().is_some_and(|audio| audio.has_playback()));
        assert_rdpsnd_playback(connector)
    }

    // oracle: mutant drop static playback when smartcard RDPDR is attached afterward
    #[test]
    fn static_playback_before_smartcard_is_preserved() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        connector.with_static_channel_rdp_snd().map_err(|_| "static playback")?;
        let rdpsnd = rdpsnd_channel_ptr(&connector)?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert_eq!(rdpsnd, rdpsnd_channel_ptr(&connector)?);
        assert_rdpsnd_playback(&connector)
    }

    // oracle: mutant drop static playback when smartcard RDPDR is attached first
    #[test]
    fn static_playback_after_smartcard_is_preserved() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        connector.with_static_channel_rdp_snd().map_err(|_| "static playback")?;
        assert_rdpsnd_playback(&connector)
    }

    // oracle: mutant drop dynamic playback when smartcard RDPDR is attached afterward
    #[test]
    fn dynamic_playback_before_smartcard_is_preserved() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        assert!(attach_direct(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        )?);
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: mutant drop dynamic playback when smartcard RDPDR is attached first
    #[test]
    fn dynamic_playback_after_smartcard_is_preserved() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert!(attach_direct(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        )?);
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: mutant drop proxy playback when smartcard RDPDR is attached afterward
    #[test]
    fn playback_proxy_before_smartcard_is_preserved() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        ));
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: mutant drop proxy playback when smartcard RDPDR is attached first
    #[test]
    fn playback_proxy_after_smartcard_is_preserved() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert!(attach_pipe_proxy(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        ));
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: mutant drop native playback when smartcard RDPDR is attached afterward
    #[test]
    fn native_playback_before_smartcard_is_preserved() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        let audio = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio")?;
        connector.with_native_audio(&audio).map_err(|_| "native audio")?;
        let rdpsnd = rdpsnd_channel_ptr(&connector)?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        assert_eq!(rdpsnd, rdpsnd_channel_ptr(&connector)?);
        assert_native_playback(&connector)
    }

    // oracle: mutant drop native playback when smartcard RDPDR is attached first
    #[test]
    fn native_playback_after_smartcard_is_preserved() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        let audio = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio")?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "smartcard")?;
        connector.with_native_audio(&audio).map_err(|_| "native audio")?;
        assert_native_playback(&connector)
    }

    #[cfg(windows)]
    fn connector_with_drive(setting: Option<bool>) -> Result<Box<ClientConnector>, &'static str> {
        let root = std::env::current_dir().map_err(|_| "current directory")?;
        let root = root.to_string_lossy();
        let mut builder = default_builder();
        builder.no_audio_playback = setting;
        builder
            .add_redirected_drive("Shared", &root, true)
            .map_err(|_| "drive")?;
        let config = builder.build().map_err(|_| "config")?;
        ClientConnector::new(&config, "127.0.0.1:0").map_err(|_| "connector")
    }

    #[cfg(windows)]
    // oracle: mutant enable audio playback when installing the drive compatibility channel
    #[test]
    fn drive_compatibility_channel_keeps_playback_disabled_by_default() -> Result<(), &'static str> {
        let connector = connector_with_drive(None)?;
        assert!(!connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    #[cfg(windows)]
    // oracle: mutant drop drive compatibility when static playback is added later
    #[test]
    fn static_playback_can_follow_drive_compatibility() -> Result<(), &'static str> {
        let mut connector = connector_with_drive(None)?;
        connector.with_static_channel_rdp_snd().map_err(|_| "static playback")?;
        assert_rdpsnd_playback(&connector)
    }

    #[cfg(windows)]
    // oracle: mutant drop drive compatibility when dynamic playback is added later
    #[test]
    fn dynamic_playback_can_follow_drive_compatibility() -> Result<(), &'static str> {
        let mut connector = connector_with_drive(None)?;
        assert!(attach_direct(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        )?);
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    #[cfg(windows)]
    // oracle: mutant drop drive compatibility when native playback is added later
    #[test]
    fn native_playback_can_follow_drive_compatibility() -> Result<(), &'static str> {
        let mut connector = connector_with_drive(None)?;
        let audio = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio")?;
        connector.with_native_audio(&audio).map_err(|_| "native audio")?;
        assert_native_playback(&connector)
    }

    #[cfg(windows)]
    // oracle: mutant ignore explicit playback enablement with a drive compatibility channel
    #[test]
    fn drive_compatibility_channel_honors_explicit_playback_enablement() -> Result<(), &'static str> {
        let connector = connector_with_drive(Some(false))?;
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    fn rdpdr_channel_ptr(connector: &ClientConnector) -> Result<*const ironrdp::rdpdr::Rdpdr, &'static str> {
        connector
            .0
            .as_ref()
            .and_then(|connector| connector.static_channels.get_by_type::<ironrdp::rdpdr::Rdpdr>())
            .and_then(|channel| channel.channel_processor_downcast_ref::<ironrdp::rdpdr::Rdpdr>())
            .map(core::ptr::from_ref)
            .ok_or("RDPDR channel")
    }

    #[derive(Debug)]
    struct TestStaticChannel(ironrdp::pdu::gcc::ChannelName);

    ironrdp_core::impl_as_any!(TestStaticChannel);

    impl ironrdp::svc::SvcProcessor for TestStaticChannel {
        fn channel_name(&self) -> ironrdp::pdu::gcc::ChannelName {
            self.0.clone()
        }

        fn process(&mut self, _payload: &[u8]) -> ironrdp::pdu::PduResult<Vec<ironrdp::svc::SvcMessage>> {
            Ok(Vec::new())
        }
    }

    impl ironrdp::svc::SvcClientProcessor for TestStaticChannel {}

    fn add_test_static_channel(connector: &mut ClientConnector, name: &str) -> Result<(), &'static str> {
        let name = ironrdp::pdu::gcc::ChannelName::from_utf8(name).ok_or("test channel name")?;
        connector
            .0
            .as_mut()
            .ok_or("connector")?
            .static_channels
            .insert_dynamic(TestStaticChannel(name))
            .ok_or("static channel capacity")?;
        Ok(())
    }

    fn add_test_static_channels(connector: &mut ClientConnector, count: usize) -> Result<(), &'static str> {
        for index in 0..count {
            add_test_static_channel(connector, &format!("C{index:06}"))?;
        }
        Ok(())
    }

    // oracle: mutant replace a foreign RDPDR channel after a rejected smartcard attachment
    #[test]
    fn foreign_rdpdr_name_collision_preserves_channels_and_connector() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        add_test_static_channel(&mut connector, "rdpdr")?;
        let channels = connector.0.as_ref().ok_or("connector")?.static_channels.len();
        let name = ironrdp::rdpdr::Rdpdr::NAME;
        let key_before = connector
            .0
            .as_ref()
            .and_then(|inner| inner.static_channels.get_by_channel_name_key(&name))
            .map(|(key, _)| key)
            .ok_or("foreign RDPDR channel")?;

        assert!(connector.with_static_channel_rdpdr("client", 42).is_err());

        let inner = connector.0.as_ref().ok_or("connector")?;
        assert_eq!(inner.static_channels.len(), channels);
        assert_eq!(
            inner.static_channels.get_by_channel_name_key(&name).map(|(key, _)| key),
            Some(key_before)
        );
        connector
            .with_static_channel_rdp_snd()
            .map_err(|_| "connector remained usable")?;
        Ok(())
    }

    // oracle: mutant mutate connector channels when RDPDR attachment lacks capacity
    #[test]
    fn insufficient_rdpdr_capacity_preserves_channels_and_connector() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        add_test_static_channels(&mut connector, ironrdp::svc::MAX_STATIC_CHANNELS - 1)?;
        let channels = connector.0.as_ref().ok_or("connector")?.static_channels.len();

        assert!(connector.with_static_channel_rdpdr("client", 42).is_err());

        assert_eq!(connector.0.as_ref().ok_or("connector")?.static_channels.len(), channels);
        connector
            .with_static_channel_rdp_snd()
            .map_err(|_| "connector remained usable")?;
        assert_eq!(
            connector.0.as_ref().ok_or("connector")?.static_channels.len(),
            ironrdp::svc::MAX_STATIC_CHANNELS
        );
        Ok(())
    }

    // oracle: mutant reject RDPDR attachment when it exactly fills static channel capacity
    #[test]
    fn rdpdr_attachment_succeeds_at_exact_static_channel_capacity() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        add_test_static_channels(&mut connector, ironrdp::svc::MAX_STATIC_CHANNELS - 2)?;

        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "RDPDR fits in the remaining capacity")?;

        assert_eq!(
            connector.0.as_ref().ok_or("connector")?.static_channels.len(),
            ironrdp::svc::MAX_STATIC_CHANNELS
        );
        Ok(())
    }

    // oracle: mutant attach RDPDR after connection negotiation has started
    #[test]
    fn late_rdpdr_attachment_preserves_connector_state() -> Result<(), &'static str> {
        let mut connector = default_connector()?;
        let mut output = crate::pdu::ffi::WriteBuf::new();
        connector.step_no_input(&mut output).map_err(|_| "start connection")?;
        let channels = connector.0.as_ref().ok_or("connector")?.static_channels.len();

        assert!(connector.with_static_channel_rdpdr("client", 42).is_err());

        let inner = connector.0.as_ref().ok_or("connector")?;
        assert_eq!(inner.static_channels.len(), channels);
        assert!(matches!(
            inner.state,
            ironrdp::connector::ClientConnectorState::ConnectionInitiationWaitConfirm { .. }
        ));
        Ok(())
    }

    fn negotiate_drive_and_smartcard_capabilities(
        rdpdr: &mut ironrdp::rdpdr::Rdpdr,
    ) -> Result<(), Box<dyn core::error::Error>> {
        use ironrdp::rdpdr::pdu::RdpdrPdu;
        use ironrdp::rdpdr::pdu::efs::{CapabilityMessage, CoreCapability, CoreCapabilityKind};
        use ironrdp::svc::SvcProcessor as _;
        use ironrdp_core::{decode, encode_vec};

        // oracle: spec MS-RDPEFS §1.3.1.1
        let server_capabilities = RdpdrPdu::CoreCapability(CoreCapability {
            capabilities: vec![
                CapabilityMessage::new_general(1),
                CapabilityMessage::new_drive(),
                CapabilityMessage::new_smartcard(),
            ],
            kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
        });
        let responses = rdpdr.process(&encode_vec(&server_capabilities)?)?;
        assert_eq!(responses.len(), 1);
        let response = responses[0].encode_unframed_pdu()?;
        let RdpdrPdu::CoreCapability(response) = decode::<RdpdrPdu>(&response)? else {
            return Err("expected client core capability response".into());
        };
        assert_eq!(response.kind, CoreCapabilityKind::ClientCoreCapabilityResponse);
        assert!(response.capabilities.contains(&CapabilityMessage::new_drive()));
        assert!(response.capabilities.contains(&CapabilityMessage::new_smartcard()));
        Ok(())
    }

    // oracle: spec MS-RDPEFS §1.3.1.1
    #[test]
    fn drive_and_smartcard_capabilities_are_exchanged() -> Result<(), Box<dyn core::error::Error>> {
        use ironrdp::rdpdr::pdu::RdpdrPdu;
        use ironrdp::rdpdr::pdu::efs::{VersionAndIdPdu, VersionAndIdPduKind};
        use ironrdp::svc::SvcProcessor as _;
        use ironrdp_core::encode_vec;

        let mut rdpdr = ironrdp::rdpdr::Rdpdr::new(Box::new(ironrdp::rdpdr::NoopRdpdrBackend), "client".to_owned())
            .with_drives(Some(vec![(1, "Shared".to_owned())]))
            .with_smartcard(42);
        let announce = RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: 1,
            version_minor: 12,
            client_id: 0x1234,
            kind: VersionAndIdPduKind::ServerAnnounceRequest,
        });
        rdpdr.process(&encode_vec(&announce)?)?;
        negotiate_drive_and_smartcard_capabilities(&mut rdpdr)
    }

    #[cfg(windows)]
    // oracle: mutant lose configured drives when smartcard RDPDR precedes native audio
    #[test]
    fn configured_drives_survive_smartcard_then_audio() -> Result<(), Box<dyn core::error::Error>> {
        assert_drives_survive_attachments("smartcard-then-audio", |connector, audio| {
            connector
                .with_static_channel_rdpdr("client", 42)
                .map_err(|_| "smartcard")?;
            connector.with_native_audio(audio).map_err(|_| "attach audio")
        })
    }

    #[cfg(windows)]
    // oracle: mutant lose configured drives when native audio precedes smartcard RDPDR
    #[test]
    fn configured_drives_survive_audio_then_smartcard() -> Result<(), Box<dyn core::error::Error>> {
        assert_drives_survive_attachments("audio-then-smartcard", |connector, audio| {
            connector.with_native_audio(audio).map_err(|_| "attach audio")?;
            connector
                .with_static_channel_rdpdr("client", 42)
                .map_err(|_| "smartcard")
        })
    }

    #[cfg(windows)]
    fn assert_drives_survive_attachments(
        test_name: &str,
        attach: impl FnOnce(&mut ClientConnector, &NativeAudio) -> Result<(), &'static str>,
    ) -> Result<(), Box<dyn core::error::Error>> {
        use ironrdp::rdpdr::pdu::RdpdrPdu;
        use ironrdp::rdpdr::pdu::efs::{
            CreateDisposition, CreateOptions, DesiredAccess, DeviceCreateRequest, DeviceIoRequest, DeviceIoResponse,
            DeviceWriteRequest, FileAttributes, MajorFunction, MinorFunction, NtStatus, ServerDeviceAnnounceResponse,
            SharedAccess, VersionAndIdPdu, VersionAndIdPduKind,
        };
        use ironrdp::svc::SvcProcessor as _;
        use ironrdp_core::{ReadCursor, WriteCursor, decode, encode_vec};

        struct TestFile(std::path::PathBuf);
        impl Drop for TestFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }

        let root = std::env::current_dir()?;
        let file_name = format!("ffi-drive-test-{}-{test_name}", std::process::id());
        let path = root.join(&file_name);
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        let _file = TestFile(path.clone());
        drop(file);
        let root = root.to_string_lossy();
        let mut builder = default_builder();
        builder
            .add_redirected_drive("ReadOnly", &root, true)
            .map_err(|_| "read-only drive")?;
        builder
            .add_redirected_drive("Writable", &root, false)
            .map_err(|_| "writable drive")?;
        let config = builder.build().map_err(|_| "configuration")?;
        let mut connector = ClientConnector::new(&config, "127.0.0.1:0").map_err(|_| "connector")?;
        let audio = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio")?;
        attach(&mut connector, &audio)?;
        let rdpdr = connector
            .0
            .as_mut()
            .ok_or("connector")?
            .get_static_channel_processor_mut::<ironrdp::rdpdr::Rdpdr>()
            .ok_or("RDPDR")?;
        let version = |kind| {
            RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
                version_major: 1,
                version_minor: 12,
                client_id: 0x1234,
                kind,
            })
        };
        rdpdr.process(&encode_vec(&version(VersionAndIdPduKind::ServerAnnounceRequest))?)?;
        negotiate_drive_and_smartcard_capabilities(rdpdr)?;
        rdpdr.process(&encode_vec(&version(VersionAndIdPduKind::ServerClientIdConfirm))?)?;
        let announcements = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon)?)?;
        let mut drives = Vec::new();
        for message in announcements {
            let bytes = message.encode_unframed_pdu()?;
            if let RdpdrPdu::ClientDeviceListAnnounce(list) = decode::<RdpdrPdu>(&bytes)? {
                for device in list.device_list {
                    drives.push((device.device_id(), device.device_data().to_vec()));
                }
            }
        }
        assert!(drives.contains(&(1, b"ReadOnly\0".to_vec())));
        assert!(drives.contains(&(2, b"Writable\0".to_vec())));
        for device_id in [1, 2] {
            let response = RdpdrPdu::ServerDeviceAnnounceResponse(ServerDeviceAnnounceResponse {
                device_id,
                result_code: NtStatus::SUCCESS,
            });
            rdpdr.process(&encode_vec(&response)?)?;
        }

        let request_header = |device_id, file_id, major_function| DeviceIoRequest {
            device_id,
            file_id,
            completion_id: 1,
            major_function,
            minor_function: MinorFunction::from(0),
        };
        for (name, device_id, access, expected_status) in [
            (
                "read-only drive rejects writes",
                1,
                DesiredAccess::GENERIC_READ,
                NtStatus::MEDIA_WRITE_PROTECTED,
            ),
            (
                "writable drive accepts writes",
                2,
                DesiredAccess::GENERIC_WRITE,
                NtStatus::SUCCESS,
            ),
        ] {
            let request = DeviceCreateRequest {
                device_io_request: request_header(device_id, 0, MajorFunction::Create),
                desired_access: access,
                allocation_size: 0,
                file_attributes: FileAttributes::empty(),
                shared_access: SharedAccess::FILE_SHARE_READ | SharedAccess::FILE_SHARE_WRITE,
                create_disposition: CreateDisposition::FILE_OPEN,
                create_options: CreateOptions::empty(),
                path: format!("\\{file_name}"),
            };
            let mut encoded = encode_vec(&RdpdrPdu::DeviceIoRequest(request.device_io_request.clone()))?;
            encoded.resize(4 + request.size(), 0);
            request.encode(&mut WriteCursor::new(&mut encoded[4..]))?;
            let messages = rdpdr.process(&encoded)?;
            let bytes = messages[0].encode_unframed_pdu()?;
            let mut cursor = ReadCursor::new(&bytes[4..]);
            let reply = DeviceIoResponse::decode(&mut cursor)?;
            assert_eq!(reply.io_status, NtStatus::SUCCESS);
            let opened = ironrdp::rdpdr::pdu::efs::DeviceCreateResponse::decode(reply, &mut cursor)?;
            let write = DeviceWriteRequest {
                device_io_request: request_header(device_id, opened.file_id, MajorFunction::Write),
                offset: 0,
                write_data: b"ok".to_vec(),
            };
            let mut encoded = encode_vec(&RdpdrPdu::DeviceIoRequest(write.device_io_request.clone()))?;
            encoded.resize(4 + write.size(), 0);
            write.encode(&mut WriteCursor::new(&mut encoded[4..]))?;
            let messages = rdpdr.process(&encoded)?;
            let bytes = messages[0].encode_unframed_pdu()?;
            let reply = DeviceIoResponse::decode(&mut ReadCursor::new(&bytes[4..]))?;
            assert_eq!(reply.io_status, expected_status, "{name}");
        }
        drop(connector);
        assert_eq!(std::fs::read(path)?, b"ok");
        Ok(())
    }

    // oracle: mutant replace an existing RDPDR channel when attaching native playback
    #[test]
    fn smartcard_rdpdr_before_native_playback_is_preserved() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "attach RDPDR")?;
        let rdpdr = rdpdr_channel_ptr(&connector)?;
        let playback = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio settings")?;

        connector
            .with_native_audio(&playback)
            .map_err(|_| "attach native playback")?;

        let inner = connector.0.as_ref().ok_or("connector")?;
        assert_eq!(rdpdr, rdpdr_channel_ptr(&connector)?);
        assert!(inner.config.enable_audio_playback);
        assert!(
            inner
                .static_channels
                .get_by_type::<ironrdp::rdpsnd::client::Rdpsnd>()
                .is_some()
        );
        Ok(())
    }

    // oracle: mutant discard native playback when attaching smartcard RDPDR
    #[test]
    fn smartcard_rdpdr_after_native_playback_preserves_playback() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let playback = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio settings")?;
        connector
            .with_native_audio(&playback)
            .map_err(|_| "attach native playback")?;
        let channel_count = connector.0.as_ref().ok_or("connector")?.static_channels.len();

        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "attach RDPDR")?;

        let inner = connector.0.as_ref().ok_or("connector")?;
        assert!(rdpdr_channel_ptr(&connector).is_ok());
        assert_eq!(channel_count, inner.static_channels.len());
        assert!(inner.config.enable_audio_playback);
        assert!(connector.1.is_some());
        assert!(
            inner
                .static_channels
                .get_by_type::<ironrdp::rdpsnd::client::Rdpsnd>()
                .is_some()
        );
        Ok(())
    }

    // oracle: mutant replace an existing smartcard RDPDR channel
    #[test]
    fn duplicate_smartcard_rdpdr_is_rejected_without_replacing_the_channel() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        connector
            .with_static_channel_rdpdr("client", 42)
            .map_err(|_| "attach RDPDR")?;
        let rdpdr = rdpdr_channel_ptr(&connector)?;

        assert!(connector.with_static_channel_rdpdr("client", 42).is_err());
        assert_eq!(rdpdr, rdpdr_channel_ptr(&connector)?);
        Ok(())
    }

    // oracle: mutant drop pre-existing playback state when attaching capture
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
    fn static_playback_before_capture_preserves_playback() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        connector.with_static_channel_rdp_snd().map_err(|_| "attach playback")?;
        let playback = core::ptr::from_ref(
            connector
                .0
                .as_ref()
                .ok_or("connector")?
                .static_channels
                .get_by_type::<ironrdp::rdpsnd::client::Rdpsnd>()
                .and_then(|channel| channel.channel_processor_downcast_ref::<ironrdp::rdpsnd::client::Rdpsnd>())
                .ok_or("playback")?,
        );
        connector
            .with_native_audio(&*capture_audio()?)
            .map_err(|_| "attach capture")?;
        let inner = connector.0.as_ref().ok_or("connector")?;
        assert!(inner.config.enable_audio_playback);
        assert!(inner.config.enable_audio_capture);
        assert_eq!(
            playback,
            core::ptr::from_ref(
                inner
                    .static_channels
                    .get_by_type::<ironrdp::rdpsnd::client::Rdpsnd>()
                    .and_then(|channel| channel.channel_processor_downcast_ref::<ironrdp::rdpsnd::client::Rdpsnd>())
                    .ok_or("playback")?
            )
        );
        Ok(())
    }

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4603169797
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
    fn static_playback_after_capture_advertises_both() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        connector
            .with_native_audio(&*capture_audio()?)
            .map_err(|_| "attach capture")?;
        connector.with_static_channel_rdp_snd().map_err(|_| "attach playback")?;
        let inner = connector.0.as_ref().ok_or("connector")?;
        assert!(inner.config.enable_audio_playback);
        assert!(inner.config.enable_audio_capture);
        assert!(
            inner
                .static_channels
                .get_by_type::<ironrdp::rdpsnd::client::Rdpsnd>()
                .is_some()
        );
        Ok(())
    }

    // oracle: mutant allow native playback alongside static playback
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
    fn existing_static_playback_rejects_native_playback_without_consuming_audio() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        connector.with_static_channel_rdp_snd().map_err(|_| "attach playback")?;
        let audio = NativeAudio::new(true, true, "", "", 0).map_err(|_| "audio settings")?;
        let channel_count = connector.0.as_ref().ok_or("connector")?.static_channels.len();

        assert!(connector.with_native_audio(&audio).is_err());
        assert!(connector.1.is_none());
        let inner = connector.0.as_ref().ok_or("connector")?;
        assert_eq!(inner.static_channels.len(), channel_count);
        assert!(inner.config.enable_audio_playback);
        assert!(!inner.config.enable_audio_capture);

        let mut other = connector_without_playback()?;
        other.with_native_audio(&audio).map_err(|_| "attach rejected audio")?;
        Ok(())
    }

    fn attach_pipe_proxy(connector: &mut ClientConnector, channel_name: &str) -> bool {
        let queue = DvcPipeProxyMessageQueue::new(1);
        let sink = queue.get_sink();
        let descriptor = DvcPipeProxyDescriptor::new(channel_name, "test-pipe");
        let mut config = DvcPipeProxyConfig::new(&sink);
        config.add_pipe_proxy(&descriptor);

        connector.with_dynamic_channel_pipe_proxy(&config).is_ok()
    }

    fn attach_direct(connector: &mut ClientConnector, channel_name: &str) -> Result<bool, &'static str> {
        let queue = DirectDvcEventQueue::new(1).map_err(|_| "direct DVC queue")?;
        let mut config = DirectDvcConfig::new(&queue.get_sink());
        config.add_channel(channel_name);
        Ok(connector.with_dynamic_channel_direct(&config).is_ok())
    }

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4604009562
    #[test]
    fn input_direct_attached_after_playback_advertises_capture() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        let playback = NativeAudio::new(true, false, "", "", 0).map_err(|_| "audio settings")?;
        connector.with_native_audio(&playback).map_err(|_| "attach playback")?;
        assert!(attach_direct(&mut connector, ironrdp_rdpeai::CHANNEL_NAME)?);
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_capture);
        Ok(())
    }

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4604009562
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
    fn playback_direct_attached_after_capture_advertises_playback() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        connector
            .with_native_audio(&*capture_audio()?)
            .map_err(|_| "attach capture")?;
        assert!(attach_direct(
            &mut connector,
            ironrdp::rdpsnd::client::DVC_CHANNEL_NAME
        )?);
        assert!(connector.0.as_ref().ok_or("connector")?.config.enable_audio_playback);
        Ok(())
    }

    // oracle: mutant allow capture to reuse a direct AUDIO_INPUT channel
    #[test]
    fn capture_and_direct_input_reject_collisions_in_both_orders() -> Result<(), &'static str> {
        let mut first = connector_without_playback()?;
        assert!(attach_direct(&mut first, ironrdp_rdpeai::CHANNEL_NAME)?);
        assert!(first.with_native_audio(&*capture_audio()?).is_err());
        let mut second = connector_without_playback()?;
        second
            .with_native_audio(&*capture_audio()?)
            .map_err(|_| "attach capture")?;
        assert!(!attach_direct(&mut second, ironrdp_rdpeai::CHANNEL_NAME)?);
        Ok(())
    }

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4602602700
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
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

    // oracle: mutant drop pre-existing playback DVC state when attaching capture
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
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

    // oracle: mutant drop pre-existing playback DVC state when attaching capture
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4604009562
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
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

    // oracle: mutant drop pre-existing capture DVC state when attaching playback
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4604009562
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

    // oracle: mutant advertise audio for an unrelated dynamic channel
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
    fn unrelated_proxy_does_not_advertise_audio() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        assert!(attach_pipe_proxy(&mut connector, "unrelated"));

        let config = &connector.0.as_ref().ok_or("connector")?.config;
        assert!(!config.enable_audio_playback);
        assert!(!config.enable_audio_capture);
        Ok(())
    }

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4603169797
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4603169797
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4602602700
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4603169803
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4604743100
    #[test]
    #[expect(deprecated, reason = "audio playback is still driven through the legacy field")]
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4604743100
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4604743100
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4604009562
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4603169803
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4603169803
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

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4603169803
    #[test]
    fn input_proxy_attached_before_capture_audio_is_rejected() -> Result<(), &'static str> {
        let mut connector = connector_without_playback()?;
        assert!(attach_pipe_proxy(&mut connector, ironrdp_rdpeai::CHANNEL_NAME));

        let capture = capture_audio()?;
        assert!(connector.with_native_audio(&capture).is_err());
        Ok(())
    }

    // oracle: regression https://github.com/Devolutions/IronRDP/pull/2469#discussion_r4602602700
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
