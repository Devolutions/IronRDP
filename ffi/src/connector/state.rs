#[diplomat::bridge]
pub mod ffi {
    use crate::{error::ffi::IronRdpError, pdu::ffi::SecurityProtocol};

    #[diplomat::opaque]
    pub struct ClientConnectorState(pub ironrdp::connector::ClientConnectorState);

    pub enum ClientConnectorStateType {
        Consumed,
        ConnectionInitiationSendRequest,
        ConnectionInitiationWaitConfirm,
        EnhancedSecurityUpgrade,
        Credssp,
        BasicSettingsExchangeSendInitial,
        BasicSettingsExchangeWaitResponse,
        ChannelConnection,
        SecureSettingsExchange,
        ConnectTimeAutoDetection,
        LicensingExchange,
        MultitransportBootstrapping,
        CapabilitiesExchange,
        ConnectionFinalization,
        Connected,
    }

    impl ClientConnectorState {
        pub fn get_type(&self) -> Result<ClientConnectorStateType, Box<IronRdpError>> {
            let res = match &self.0 {
                ironrdp::connector::ClientConnectorState::Consumed => ClientConnectorStateType::Consumed,
                ironrdp::connector::ClientConnectorState::ConnectionInitiationSendRequest => {
                    ClientConnectorStateType::ConnectionInitiationSendRequest
                }
                ironrdp::connector::ClientConnectorState::ConnectionInitiationWaitConfirm { .. } => {
                    ClientConnectorStateType::ConnectionInitiationWaitConfirm
                }
                ironrdp::connector::ClientConnectorState::EnhancedSecurityUpgrade { .. } => {
                    ClientConnectorStateType::EnhancedSecurityUpgrade
                }
                ironrdp::connector::ClientConnectorState::Credssp { .. } => ClientConnectorStateType::Credssp,
                ironrdp::connector::ClientConnectorState::BasicSettingsExchangeSendInitial { .. } => {
                    ClientConnectorStateType::BasicSettingsExchangeSendInitial
                }
                ironrdp::connector::ClientConnectorState::BasicSettingsExchangeWaitResponse { .. } => {
                    ClientConnectorStateType::BasicSettingsExchangeWaitResponse
                }
                ironrdp::connector::ClientConnectorState::ChannelConnection { .. } => {
                    ClientConnectorStateType::ChannelConnection
                }
                ironrdp::connector::ClientConnectorState::SecureSettingsExchange { .. } => {
                    ClientConnectorStateType::SecureSettingsExchange
                }
                ironrdp::connector::ClientConnectorState::ConnectTimeAutoDetection { .. } => {
                    ClientConnectorStateType::ConnectTimeAutoDetection
                }
                ironrdp::connector::ClientConnectorState::LicensingExchange { .. } => {
                    ClientConnectorStateType::LicensingExchange
                }
                ironrdp::connector::ClientConnectorState::MultitransportBootstrapping { .. } => {
                    ClientConnectorStateType::MultitransportBootstrapping
                }
                ironrdp::connector::ClientConnectorState::CapabilitiesExchange { .. } => {
                    ClientConnectorStateType::CapabilitiesExchange
                }
                ironrdp::connector::ClientConnectorState::ConnectionFinalization { .. } => {
                    ClientConnectorStateType::ConnectionFinalization
                }
                ironrdp::connector::ClientConnectorState::Connected { .. } => ClientConnectorStateType::Connected,
                &_ => return Err("Unknown ClientConnectorStateType".into()),
            };

            Ok(res)
        }

        pub fn get_connection_initiation_wait_confirm_requested_protocol(
            &self,
        ) -> Result<Box<SecurityProtocol>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::ClientConnectorState::ConnectionInitiationWaitConfirm { requested_protocol } => {
                    Ok(SecurityProtocol(*requested_protocol))
                }
                _ => Err("Not in ConnectionInitiationWaitConfirm state".into()),
            }
            .map(Box::new)
        }

        pub fn get_enhanced_security_upgrade_selected_protocol(
            &self,
        ) -> Result<Box<SecurityProtocol>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol } => {
                    Ok(SecurityProtocol(*selected_protocol))
                }
                _ => Err("Not in EnhancedSecurityUpgrade state".into()),
            }
            .map(Box::new)
        }

        pub fn get_credssp_selected_protocol(
            &self,
        ) -> Result<Box<SecurityProtocol>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::ClientConnectorState::Credssp { selected_protocol } => {
                    Ok(SecurityProtocol(*selected_protocol))
                }
                _ => Err("Not in Credssp state".into()),
            }
            .map(Box::new)
        }

        pub fn get_basic_settings_exchange_send_initial_selected_protocol(
            &self,
        ) -> Result<Box<SecurityProtocol>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol } => {
                    Ok(SecurityProtocol(*selected_protocol))
                }
                _ => Err("Not in BasicSettingsExchangeSendInitial state".into()),
            }
            .map(Box::new)
        }

        pub fn get_basic_settings_exchange_wait_response_connect_initial(
            &self,
        ) -> Result<Box<crate::pdu::ffi::ConnectInitial>, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::ClientConnectorState::BasicSettingsExchangeWaitResponse { connect_initial } => {
                    Ok(crate::pdu::ffi::ConnectInitial(connect_initial.clone()))
                }
                _ => Err("Not in BasicSettingsExchangeWaitResponse state".into()),
            }
            .map(Box::new)
        }

        pub fn get_channel_connection_io_channel_id(&self) -> Result<u16, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::ClientConnectorState::ChannelConnection { io_channel_id, .. } => Ok(*io_channel_id),
                _ => Err("Not in ChannelConnection state".into()),
            }
        }

        pub fn get_secure_settings_exchange_io_channel_id(&self) -> Result<u16, Box<IronRdpError>> {
            match &self.0 {
                ironrdp::connector::ClientConnectorState::SecureSettingsExchange { io_channel_id, .. } => {
                    Ok(*io_channel_id)
                }
                _ => Err("Not in SecureSettingsExchange state".into()),
            }
        }

        // TODO: Add more getters for other states

        pub fn get_connected_result<'a>(
            &'a self,
        ) -> Result<Box<crate::connector::result::ffi::ConnectionResult<'a>>, Box<IronRdpError>>
        {
            match &self.0 {
                ironrdp::connector::ClientConnectorState::Connected { result } => Ok(Box::new(
                    crate::connector::result::ffi::ConnectionResult(result),
                )),
                _ => Err("Not in Connected state".into()),
            }
        }
    }

}
