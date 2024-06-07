#[diplomat::bridge]
pub mod ffi {
    use crate::{
        error::{ffi::IronRdpError, IncorrectEnumTypeError, ValueConsumedError},
        pdu::ffi::SecurityProtocol,
    };

    #[diplomat::opaque]
    pub struct ClientConnectorState(pub Option<ironrdp::connector::ClientConnectorState>);

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
        pub fn get_enum_type(&self) -> Result<ClientConnectorStateType, Box<IronRdpError>> {
            let res = match &self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("ClientConnectorState"))?
            {
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
            &mut self,
        ) -> Result<Box<SecurityProtocol>, Box<IronRdpError>> {
            match self
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("ClientConnectorState"))?
            {
                ironrdp::connector::ClientConnectorState::ConnectionInitiationWaitConfirm { requested_protocol } => {
                    Ok(SecurityProtocol(requested_protocol))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("ConnectionInitiationWaitConfirm")
                    .of_enum("ClientConnectorState")
                    .into()),
            }
            .map(Box::new)
        }

        pub fn get_enhanced_security_upgrade_selected_protocol(
            &mut self,
        ) -> Result<Box<SecurityProtocol>, Box<IronRdpError>> {
            match self
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("ClientConnectorState"))?
            {
                ironrdp::connector::ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol } => {
                    Ok(SecurityProtocol(selected_protocol))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("EnhancedSecurityUpgrade")
                    .of_enum("ClientConnectorState")
                    .into()),
            }
            .map(Box::new)
        }

        pub fn get_credssp_selected_protocol(&mut self) -> Result<Box<SecurityProtocol>, Box<IronRdpError>> {
            match self
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("ClientConnectorState"))?
            {
                ironrdp::connector::ClientConnectorState::Credssp { selected_protocol } => {
                    Ok(SecurityProtocol(selected_protocol))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("Credssp")
                    .of_enum("ClientConnectorState")
                    .into()),
            }
            .map(Box::new)
        }

        pub fn get_basic_settings_exchange_send_initial_selected_protocol(
            &mut self,
        ) -> Result<Box<SecurityProtocol>, Box<IronRdpError>> {
            match self
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("ClientConnectorState"))?
            {
                ironrdp::connector::ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol } => {
                    Ok(SecurityProtocol(selected_protocol))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("BasicSettingsExchangeSendInitial")
                    .of_enum("ClientConnectorState")
                    .into()),
            }
            .map(Box::new)
        }

        pub fn get_basic_settings_exchange_wait_response_connect_initial(
            &mut self,
        ) -> Result<Box<crate::pdu::ffi::ConnectInitial>, Box<IronRdpError>> {
            match self
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("ClientConnectorState"))?
            {
                ironrdp::connector::ClientConnectorState::BasicSettingsExchangeWaitResponse { connect_initial } => {
                    Ok(crate::pdu::ffi::ConnectInitial(connect_initial))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("BasicSettingsExchangeWaitResponse")
                    .of_enum("ClientConnectorState")
                    .into()),
            }
            .map(Box::new)
        }

        pub fn get_connected_result(
            &mut self,
        ) -> Result<Box<crate::connector::result::ffi::ConnectionResult>, Box<IronRdpError>> {
            match self
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("ClientConnectorState"))?
            {
                ironrdp::connector::ClientConnectorState::Connected { result } => {
                    Ok(Box::new(crate::connector::result::ffi::ConnectionResult(Some(result))))
                }
                _ => Err(IncorrectEnumTypeError::on_variant("Connected")
                    .of_enum("ClientConnectorState")
                    .into()),
            }
        }

        pub fn get_connection_finalization_result(
            &mut self,
        ) -> Result<Box<crate::connector::activation::ffi::ConnectionActivationSequence>, Box<IronRdpError>> {
            match self
                .0
                .take()
                .ok_or_else(|| ValueConsumedError::for_item("ClientConnectorState"))?
            {
                ironrdp::connector::ClientConnectorState::ConnectionFinalization { connection_activation } => Ok(
                    crate::connector::activation::ffi::ConnectionActivationSequence(Box::new(connection_activation)),
                ),
                _ => Err(IncorrectEnumTypeError::on_variant("ConnectionFinalization")
                    .of_enum("ClientConnectorState")
                    .into()),
            }
            .map(Box::new)
        }
    }
}
