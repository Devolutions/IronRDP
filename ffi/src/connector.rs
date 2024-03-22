macro_rules! take_if_not_none {
    ($expr:expr) => {{
        if let Some(value) = $expr.take() {
            value
        } else {
            panic!("Inner value is None")
        }
    }};
}

#[diplomat::bridge]
pub mod ffi {
    use diplomat_runtime::DiplomatWriteable;
    use ironrdp::connector::Sequence as _;
    use std::fmt::Write;

    use crate::{
        error::ffi::IronRdpError,
        utils::ffi::{SocketAddr, VecU8},
    };

    #[diplomat::opaque] // We must use Option here, as ClientConnector is not Clone and have functions that consume it
    pub struct ClientConnector(pub Option<ironrdp::connector::ClientConnector>);

    #[diplomat::opaque]
    pub struct Config(pub ironrdp::connector::Config);

    #[diplomat::opaque]
    pub struct ClientConnectorState(pub ironrdp::connector::ClientConnectorState);

    // Basic Impl for ClientConnector
    impl ClientConnector {
        pub fn new(config: &Config) -> Box<ClientConnector> {
            Box::new(ClientConnector(Some(ironrdp::connector::ClientConnector::new(
                config.0.clone(),
            ))))
        }

        /// Must use
        pub fn with_server_addr(&mut self, server_addr: &SocketAddr) {
            let connector = take_if_not_none!(self.0);
            let server_addr = server_addr.0.clone();
            self.0 = Some(connector.with_server_addr(server_addr));
        }

        // FIXME: We need to create opaque for ironrdp::svc::StaticChannelSet
        /// Must use
        pub fn with_static_channel_rdp_snd(&mut self) {
            let connector = take_if_not_none!(self.0);
            self.0 = Some(connector.with_static_channel(ironrdp::rdpsnd::Rdpsnd::new()));
        }

        // FIXME: We need to create opaque for ironrdp::rdpdr::Rdpdr
        /// Must use
        pub fn with_static_channel_rdpdr(&mut self, computer_name: &str, smart_card_device_id: u32) {
            let connector = take_if_not_none!(self.0);
            self.0 = Some(
                connector.with_static_channel(
                    ironrdp::rdpdr::Rdpdr::new(
                        Box::new(ironrdp::rdpdr::NoopRdpdrBackend {}),
                        computer_name.to_string(),
                    )
                    .with_smartcard(smart_card_device_id),
                ),
            );
        }

        pub fn should_perform_security_upgrade(&self) -> bool {
            let Some(connector) = self.0.as_ref() else {
                panic!("Inner value is None")
            };
            connector.should_perform_security_upgrade()
        }

        pub fn mark_security_upgrade_as_done(&mut self) {
            let Some(connector) = self.0.as_mut() else {
                panic!("Inner value is None")
            };
            connector.mark_security_upgrade_as_done();
        }

        pub fn should_perform_credssp(&self) -> bool {
            let Some(connector) = self.0.as_ref() else {
                panic!("Inner value is None")
            };
            connector.should_perform_credssp()
        }

        pub fn mark_credssp_as_done(&mut self) {
            let Some(connector) = self.0.as_mut() else {
                panic!("Inner value is None")
            };
            connector.mark_credssp_as_done();
        }
    }

    #[diplomat::opaque]
    pub struct PduHintResult<'a>(pub Option<&'a dyn ironrdp::pdu::PduHint>);

    impl<'a> PduHintResult<'a> {
        pub fn is_some(&'a self) -> bool {
            self.0.is_some()
        }

        pub fn find_size(&'a self, buffer: &VecU8) -> Result<Option<usize>, Box<IronRdpError>> {
            let Some(pdu_hint) = self.0 else {
                return Ok(None);
            };

            let size = pdu_hint.find_size(buffer.0.as_slice())?;

            Ok(size)
        }
    }

    #[diplomat::opaque]
    pub struct State<'a>(pub &'a dyn ironrdp::connector::State);

    impl<'a> State<'a> {
        pub fn get_name(&'a self,writeable:&'a mut DiplomatWriteable) -> Result<(), Box<IronRdpError>>{
            let name = self.0.name();
            write!(writeable, "{}", name)?;
            Ok(())
        }

        pub fn is_terminal(&'a self) -> bool {
            self.0.is_terminal()
        }

        pub fn as_any(&'a self) -> Box<crate::utils::ffi::Any> {
            Box::new(crate::utils::ffi::Any(self.0.as_any()))
        }
        
    }


    impl ClientConnector {
        pub fn next_pdu_hint(&self) -> Box<PduHintResult> {
            let Some(connector) = self.0.as_ref() else {
                panic!("Inner value is None")
            };
            Box::new(PduHintResult(connector.next_pdu_hint()))
        }

        pub fn state(&self) -> Box<State> {
            let Some(connector) = self.0.as_ref() else {
                panic!("Inner value is None")
            };
            Box::new(State(connector.state()))
        }
    }

}
