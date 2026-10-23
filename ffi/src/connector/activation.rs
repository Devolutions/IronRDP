#[diplomat::bridge]
pub mod ffi {
    use ironrdp::connector::Sequence as _;

    use crate::connector::config::ffi::DesktopSize;
    use crate::connector::ffi::PduHint;
    use crate::connector::result::ffi::Written;
    use crate::error::IncorrectEnumTypeError;
    use crate::error::ffi::IronRdpError;
    use crate::pdu::ffi::WriteBuf;

    #[diplomat::opaque]
    pub struct ConnectionActivationSequence(
        pub Box<ironrdp::connector::connection_activation::ConnectionActivationSequence>,
    );

    impl ConnectionActivationSequence {
        #[diplomat::attr(auto, getter = "state")]
        pub fn get_state(&self) -> Box<ConnectionActivationState> {
            Box::new(ConnectionActivationState {
                state: self.0.connection_activation_state(),
            })
        }

        pub fn next_pdu_hint<'a>(&'a self) -> Result<Option<Box<PduHint<'a>>>, Box<IronRdpError>> {
            let pdu_hint = self.0.next_pdu_hint();
            Ok(pdu_hint.map(PduHint).map(Box::new))
        }

        pub fn step(&mut self, pdu_hint: &[u8], buf: &mut WriteBuf) -> Result<Box<Written>, Box<IronRdpError>> {
            let res = self.0.step(pdu_hint, None, &mut buf.0).map(Written).map(Box::new)?;
            Ok(res)
        }

        pub fn step_no_input(&mut self, buf: &mut WriteBuf) -> Result<Box<Written>, Box<IronRdpError>> {
            let res = self.0.step_no_input(&mut buf.0).map(Written).map(Box::new)?;
            Ok(res)
        }

        #[diplomat::attr(auto, getter = "io_channel_id")]
        pub fn get_io_channel_id(&self) -> u16 {
            self.0.io_channel_id()
        }

        #[diplomat::attr(auto, getter = "user_channel_id")]
        pub fn get_user_channel_id(&self) -> u16 {
            self.0.user_channel_id()
        }
    }

    #[diplomat::opaque]
    pub struct ConnectionActivationState {
        pub state: ironrdp::connector::connection_activation::ConnectionActivationState,
    }

    pub enum ConnectionActivationStateType {
        Consumed,
        CapabilitiesExchange,
        ConnectionFinalization,
        Finalized,
        Redirected,
    }

    impl ConnectionActivationState {
        #[diplomat::attr(auto, getter = "type")]
        pub fn get_type(&self) -> ConnectionActivationStateType {
            match self.state {
                ironrdp::connector::connection_activation::ConnectionActivationState::Consumed => {
                    ConnectionActivationStateType::Consumed
                }
                ironrdp::connector::connection_activation::ConnectionActivationState::CapabilitiesExchange => {
                    ConnectionActivationStateType::CapabilitiesExchange
                }
                ironrdp::connector::connection_activation::ConnectionActivationState::ConnectionFinalization {
                    ..
                } => ConnectionActivationStateType::ConnectionFinalization,
                ironrdp::connector::connection_activation::ConnectionActivationState::Finalized { .. } => {
                    ConnectionActivationStateType::Finalized
                }
                ironrdp::connector::connection_activation::ConnectionActivationState::Redirected { .. } => {
                    ConnectionActivationStateType::Redirected
                }
            }
        }

        #[diplomat::attr(auto, getter = "connection_finalization")]
        pub fn get_connection_finalization(
            &self,
        ) -> Result<Box<ConnectionActivationStateConnectionFinalization>, Box<IronRdpError>> {
            match &self.state {
                ironrdp::connector::connection_activation::ConnectionActivationState::ConnectionFinalization {
                    desktop_size,
                    share_id: _,
                    input_flags: _,
                    connection_finalization,
                    ..
                } => Ok(Box::new(ConnectionActivationStateConnectionFinalization {
                    desktop_size: *desktop_size,
                    connection_finalization: connection_finalization.clone(),
                })),
                _ => Err(IncorrectEnumTypeError::on_variant("ConnectionFinalization")
                    .of_enum("ConnectionActivationState")
                    .into()),
            }
        }

        #[diplomat::attr(auto, getter = "redirected")]
        pub fn get_redirected(
            &self,
        ) -> Result<Box<crate::connector::result::ffi::ServerRedirection>, Box<IronRdpError>> {
            match &self.state {
                ironrdp::connector::connection_activation::ConnectionActivationState::Redirected { redirection } => {
                    Ok(Box::new(crate::connector::result::ffi::ServerRedirection(
                        redirection.as_ref().clone(),
                    )))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("Redirected")
                    .of_enum("ConnectionActivationState")
                    .into()),
            }
        }

        #[diplomat::attr(auto, getter = "finalized")]
        pub fn get_finalized(&self) -> Result<Box<ConnectionActivationStateFinalized>, Box<IronRdpError>> {
            match &self.state {
                ironrdp::connector::connection_activation::ConnectionActivationState::Finalized {
                    desktop_size,
                    share_id,
                    input_flags: _,
                    enable_server_pointer,
                    pointer_software_rendering,
                    static_channel_chunk_size,
                    window_support_level,
                    ..
                } => Ok(Box::new(ConnectionActivationStateFinalized {
                    share_id: *share_id,
                    desktop_size: *desktop_size,
                    enable_server_pointer: *enable_server_pointer,
                    pointer_software_rendering: *pointer_software_rendering,
                    static_channel_chunk_size: *static_channel_chunk_size,
                    window_support_level: *window_support_level,
                })),
                _ => Err(IncorrectEnumTypeError::on_variant("Finalized")
                    .of_enum("ConnectionActivationState")
                    .into()),
            }
        }
    }

    #[diplomat::opaque]
    pub struct ConnectionActivationStateConnectionFinalization {
        pub desktop_size: ironrdp::connector::DesktopSize,
        pub connection_finalization: ironrdp::connector::ConnectionFinalizationSequence,
    }

    impl ConnectionActivationStateConnectionFinalization {
        #[diplomat::attr(auto, getter = "desktop_size")]
        pub fn get_desktop_size(&self) -> Box<DesktopSize> {
            Box::new(DesktopSize(self.desktop_size))
        }
    }

    #[diplomat::opaque]
    pub struct ConnectionActivationStateFinalized {
        pub share_id: u32,
        pub desktop_size: ironrdp::connector::DesktopSize,
        pub enable_server_pointer: bool,
        pub pointer_software_rendering: bool,
        pub static_channel_chunk_size: usize,
        pub window_support_level: Option<ironrdp::pdu::rdp::capability_sets::WindowSupportLevel>,
    }

    impl ConnectionActivationStateFinalized {
        #[diplomat::attr(auto, getter = "share_id")]
        pub fn get_share_id(&self) -> u32 {
            self.share_id
        }

        #[diplomat::attr(auto, getter = "desktop_size")]
        pub fn get_desktop_size(&self) -> Box<DesktopSize> {
            Box::new(DesktopSize(self.desktop_size))
        }

        #[diplomat::attr(auto, getter = "enable_server_pointer")]
        pub fn get_enable_server_pointer(&self) -> bool {
            self.enable_server_pointer
        }

        #[diplomat::attr(auto, getter = "pointer_software_rendering")]
        pub fn get_pointer_software_rendering(&self) -> bool {
            self.pointer_software_rendering
        }

        #[diplomat::attr(auto, getter = "static_channel_chunk_size")]
        pub fn get_static_channel_chunk_size(&self) -> usize {
            self.static_channel_chunk_size
        }

        /// Returns -1 when Window List support was not negotiated, otherwise
        /// the negotiated Window List support level.
        #[diplomat::attr(auto, getter = "window_support_level")]
        pub fn get_window_support_level(&self) -> i8 {
            match self.window_support_level {
                None => -1,
                Some(ironrdp::pdu::rdp::capability_sets::WindowSupportLevel::Supported) => 1,
                Some(ironrdp::pdu::rdp::capability_sets::WindowSupportLevel::SupportedEx) => 2,
                Some(ironrdp::pdu::rdp::capability_sets::WindowSupportLevel::NotSupported) => 0,
            }
        }
    }
}
