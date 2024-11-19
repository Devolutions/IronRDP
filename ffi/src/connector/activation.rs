#[diplomat::bridge]
pub mod ffi {
    use ironrdp::connector::Sequence;

    use crate::connector::config::ffi::DesktopSize;
    use crate::connector::ffi::PduHint;
    use crate::connector::result::ffi::Written;
    use crate::error::ffi::IronRdpError;
    use crate::error::IncorrectEnumTypeError;
    use crate::pdu::ffi::WriteBuf;

    #[diplomat::opaque]
    pub struct ConnectionActivationSequence(
        pub Box<ironrdp::connector::connection_activation::ConnectionActivationSequence>,
    );

    impl ConnectionActivationSequence {
        pub fn get_state(&self) -> Box<ConnectionActivationState> {
            Box::new(ConnectionActivationState(self.0.state.clone()))
        }

        pub fn next_pdu_hint<'a>(&'a self) -> Result<Option<Box<PduHint<'a>>>, Box<IronRdpError>> {
            let pdu_hint = self.0.next_pdu_hint();
            Ok(pdu_hint.map(PduHint).map(Box::new))
        }

        pub fn step(&mut self, pdu_hint: &[u8], buf: &mut WriteBuf) -> Result<Box<Written>, Box<IronRdpError>> {
            let res = self.0.step(pdu_hint, &mut buf.0).map(Written).map(Box::new)?;
            Ok(res)
        }

        pub fn step_no_input(&mut self, buf: &mut WriteBuf) -> Result<Box<Written>, Box<IronRdpError>> {
            let res = self.0.step_no_input(&mut buf.0).map(Written).map(Box::new)?;
            Ok(res)
        }
    }

    #[diplomat::opaque]
    pub struct ConnectionActivationState(pub ironrdp::connector::connection_activation::ConnectionActivationState);

    pub enum ConnectionActivationStateType {
        Consumed,
        CapabilitiesExchange,
        ConnectionFinalization,
        Finalized,
    }

    impl ConnectionActivationState {
        pub fn get_type(&self) -> ConnectionActivationStateType {
            match self.0 {
                ironrdp::connector::connection_activation::ConnectionActivationState::Consumed => {
                    ConnectionActivationStateType::Consumed
                }
                ironrdp::connector::connection_activation::ConnectionActivationState::CapabilitiesExchange {
                    ..
                } => ConnectionActivationStateType::CapabilitiesExchange,
                ironrdp::connector::connection_activation::ConnectionActivationState::ConnectionFinalization {
                    ..
                } => ConnectionActivationStateType::ConnectionFinalization,
                ironrdp::connector::connection_activation::ConnectionActivationState::Finalized { .. } => {
                    ConnectionActivationStateType::Finalized
                }
            }
        }

        pub fn get_capabilities_exchange(
            &self,
        ) -> Result<Box<ConnectionActivationStateCapabilitiesExchange>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::connection_activation::ConnectionActivationState::CapabilitiesExchange {
                    io_channel_id,
                    user_channel_id,
                } => Ok(Box::new(ConnectionActivationStateCapabilitiesExchange {
                    io_channel_id: *io_channel_id,
                    user_channel_id: *user_channel_id,
                })),
                _ => Err(IncorrectEnumTypeError::on_variant("CapabilitiesExchange")
                    .of_enum("ConnectionActivationState")
                    .into()),
            }
        }

        pub fn get_connection_finalization(
            &self,
        ) -> Result<Box<ConnectionActivationStateConnectionFinalization>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::connection_activation::ConnectionActivationState::ConnectionFinalization {
                    io_channel_id,
                    user_channel_id,
                    desktop_size,
                    connection_finalization,
                } => Ok(Box::new(ConnectionActivationStateConnectionFinalization {
                    io_channel_id: *io_channel_id,
                    user_channel_id: *user_channel_id,
                    desktop_size: *desktop_size,
                    connection_finalization: connection_finalization.clone(),
                })),
                _ => Err(IncorrectEnumTypeError::on_variant("ConnectionFinalization")
                    .of_enum("ConnectionActivationState")
                    .into()),
            }
        }

        pub fn get_finalized(&self) -> Result<Box<ConnectionActivationStateFinalized>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::connection_activation::ConnectionActivationState::Finalized {
                    io_channel_id,
                    user_channel_id,
                    desktop_size,
                    no_server_pointer,
                    pointer_software_rendering,
                } => Ok(Box::new(ConnectionActivationStateFinalized {
                    io_channel_id: *io_channel_id,
                    user_channel_id: *user_channel_id,
                    desktop_size: *desktop_size,
                    no_server_pointer: *no_server_pointer,
                    pointer_software_rendering: *pointer_software_rendering,
                })),
                _ => Err(IncorrectEnumTypeError::on_variant("Finalized")
                    .of_enum("ConnectionActivationState")
                    .into()),
            }
        }
    }

    #[diplomat::opaque]
    pub struct ConnectionActivationStateCapabilitiesExchange {
        pub io_channel_id: u16,
        pub user_channel_id: u16,
    }

    impl ConnectionActivationStateCapabilitiesExchange {
        pub fn get_io_channel_id(&self) -> u16 {
            self.io_channel_id
        }

        pub fn get_user_channel_id(&self) -> u16 {
            self.user_channel_id
        }
    }

    #[diplomat::opaque]
    pub struct ConnectionActivationStateConnectionFinalization {
        pub io_channel_id: u16,
        pub user_channel_id: u16,
        pub desktop_size: ironrdp::connector::DesktopSize,
        pub connection_finalization: ironrdp::connector::ConnectionFinalizationSequence,
    }

    impl ConnectionActivationStateConnectionFinalization {
        pub fn get_io_channel_id(&self) -> u16 {
            self.io_channel_id
        }

        pub fn get_user_channel_id(&self) -> u16 {
            self.user_channel_id
        }

        pub fn get_desktop_size(&self) -> Box<DesktopSize> {
            Box::new(DesktopSize(self.desktop_size))
        }
    }

    #[diplomat::opaque]
    pub struct ConnectionActivationStateFinalized {
        pub io_channel_id: u16,
        pub user_channel_id: u16,
        pub desktop_size: ironrdp::connector::DesktopSize,
        pub no_server_pointer: bool,
        pub pointer_software_rendering: bool,
    }

    impl ConnectionActivationStateFinalized {
        pub fn get_io_channel_id(&self) -> u16 {
            self.io_channel_id
        }

        pub fn get_user_channel_id(&self) -> u16 {
            self.user_channel_id
        }

        pub fn get_desktop_size(&self) -> Box<DesktopSize> {
            Box::new(DesktopSize(self.desktop_size))
        }

        pub fn get_no_server_pointer(&self) -> bool {
            self.no_server_pointer
        }

        pub fn get_pointer_software_rendering(&self) -> bool {
            self.pointer_software_rendering
        }
    }
}
