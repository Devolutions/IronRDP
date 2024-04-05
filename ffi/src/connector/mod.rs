pub mod config;
pub mod result;
pub mod state;

#[diplomat::bridge]
pub mod ffi {
    use diplomat_runtime::DiplomatWriteable;
    use ironrdp::connector::Sequence as _;
    use std::fmt::Write;

    use crate::{
        error::{
            ffi::{IronRdpError, IronRdpErrorKind},
            ValueConsumedError,
        },
        pdu::ffi::WriteBuf,
    };

    use super::{config::ffi::Config, result::ffi::Written};

    #[diplomat::opaque] // We must use Option here, as ClientConnector is not Clone and have functions that consume it
    pub struct ClientConnector(pub Option<ironrdp::connector::ClientConnector>);

    // Basic Impl for ClientConnector
    impl ClientConnector {
        pub fn new(config: &Config) -> Box<ClientConnector> {
            Box::new(ClientConnector(Some(ironrdp::connector::ClientConnector::new(
                config.0.clone(),
            ))))
        }

        /// Must use
        pub fn with_server_addr(&mut self, server_addr: &str) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.take() else {
                return Err(IronRdpErrorKind::Consumed.into());
            };
            let server_addr = server_addr.parse().map_err(|_| IronRdpErrorKind::Generic)?;
            self.0 = Some(connector.with_server_addr(server_addr));

            Ok(())
        }

        // FIXME: We need to create opaque for ironrdp::svc::StaticChannelSet
        /// Must use
        pub fn with_static_channel_rdp_snd(&mut self) -> Result<(), Box<IronRdpError>> {
            let Some(connector) = self.0.take() else {
                return Err(IronRdpErrorKind::Consumed.into());
            };

            self.0 = Some(connector.with_static_channel(ironrdp::rdpsnd::Rdpsnd::new()));

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
    }

    #[diplomat::opaque]
    pub struct PduHint<'a>(pub &'a dyn ironrdp::pdu::PduHint);

    impl<'a> PduHint<'a> {
        pub fn find_size(&'a self, bytes: &[u8]) -> Result<Box<crate::utils::ffi::OptionalUsize>, Box<IronRdpError>> {
            let pdu_hint = self.0;
            let size = pdu_hint.find_size(bytes)?;
            Ok(Box::new(crate::utils::ffi::OptionalUsize(size)))
        }
    }

    #[diplomat::opaque]
    pub struct State<'a>(pub &'a dyn ironrdp::connector::State);

    impl<'a> State<'a> {
        pub fn get_name(&'a self, writeable: &'a mut DiplomatWriteable) -> Result<(), Box<IronRdpError>> {
            let name = self.0.name();
            write!(writeable, "{}", name)?;
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
            Ok(connector.next_pdu_hint().map(PduHint).map(Box::new))
        }

        pub fn state(&self) -> Result<Box<State<'_>>, Box<IronRdpError>> {
            let Some(connector) = self.0.as_ref() else {
                return Err(ValueConsumedError::for_item("connector").into());
            };
            Ok(Box::new(State(connector.state())))
        }
    }

    #[diplomat::opaque]
    pub struct ChannelConnectionSequence(pub ironrdp::connector::ChannelConnectionSequence);

    #[diplomat::opaque]
    pub struct LicenseExchangeSequence(pub ironrdp::connector::LicenseExchangeSequence);

    #[diplomat::opaque]
    pub struct ConnectionActivationSequence(pub ironrdp::connector::connection_activation::ConnectionActivationSequence);
}
