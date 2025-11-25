use core::mem;

use ironrdp_connector::{
    general_err, reason_err, ClientConnector, ClientConnectorState, ConnectorError, ConnectorErrorExt as _,
    ConnectorResult, CredsspSequenceFactory, Sequence, State, Written,
};
use ironrdp_core::{decode, WriteBuf};
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::{nego, PduHint};
use tracing::{debug, error, info};

use crate::config::VmConnectorConfig;

pub const HYPERV_SECURITY_PROTOCOL: SecurityProtocol = SecurityProtocol::HYBRID_EX
    .union(SecurityProtocol::SSL)
    .union(SecurityProtocol::HYBRID);

#[derive(Default, Debug)]
#[non_exhaustive]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum VmConnectorState {
    #[default]
    Consumed,
    EnhancedSecurityUpgrade,
    Credssp,
    ConnectionInitiationSendRequest,
    ConnectionInitiationWaitConfirm,
    Handover {
        selected_protocol: SecurityProtocol,
    },
}

impl State for VmConnectorState {
    fn name(&self) -> &'static str {
        match self {
            Self::Consumed => "Consumed",
            Self::ConnectionInitiationSendRequest => "ConnectionInitiationSendRequest",
            Self::ConnectionInitiationWaitConfirm => "ConnectionInitiationWaitResponse",
            Self::EnhancedSecurityUpgrade => "EnhancedSecurityUpgrade",
            Self::Credssp => "Credssp",
            Self::Handover { .. } => "Handover",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Handover { .. })
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct VmClientConnector {
    config: VmConnectorConfig,
    state: VmConnectorState,
    client_connector: ClientConnector, // hold it hostage, can't do anything with it until VMConnector handover
}

impl Sequence for VmClientConnector {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match &self.state {
            VmConnectorState::Consumed => None,
            VmConnectorState::ConnectionInitiationSendRequest => None,
            VmConnectorState::ConnectionInitiationWaitConfirm => Some(&ironrdp_pdu::X224_HINT),
            VmConnectorState::EnhancedSecurityUpgrade => None,
            VmConnectorState::Credssp => None,
            VmConnectorState::Handover { .. } => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            // Invalid state
            VmConnectorState::Consumed => {
                return Err(general_err!("connector sequence state is consumed (this is a bug)",))
            }

            //== Connection Initiation ==//
            // Exchange supported security protocols and a few other connection flags.
            VmConnectorState::EnhancedSecurityUpgrade => (Written::Nothing, VmConnectorState::Credssp),

            VmConnectorState::Credssp => (Written::Nothing, VmConnectorState::ConnectionInitiationSendRequest),
            VmConnectorState::ConnectionInitiationSendRequest => {
                debug!("Connection Initiation");

                let connection_request = nego::ConnectionRequest {
                    nego_data: self
                        .config
                        .request_data
                        .clone()
                        .or_else(|| Some(nego::NegoRequestData::cookie(self.config.credentials.username.clone()))),
                    flags: nego::RequestFlags::empty(),
                    protocol: HYPERV_SECURITY_PROTOCOL,
                };

                debug!(message = ?connection_request, "Send");

                let written =
                    ironrdp_core::encode_buf(&X224(connection_request), output).map_err(ConnectorError::encode)?;

                (
                    Written::from_size(written)?,
                    VmConnectorState::ConnectionInitiationWaitConfirm,
                )
            }
            VmConnectorState::ConnectionInitiationWaitConfirm => {
                let connection_confirm = decode::<X224<nego::ConnectionConfirm>>(input)
                    .map_err(ConnectorError::decode)
                    .map(|p| p.0)?;

                debug!(message = ?connection_confirm, "Received");

                let (flags, selected_protocol) = match connection_confirm {
                    nego::ConnectionConfirm::Response { flags, protocol } => (flags, protocol),
                    nego::ConnectionConfirm::Failure { code } => {
                        error!(?code, "Received connection failure code");
                        return Err(reason_err!("Initiation", "{code}"));
                    }
                };

                info!(?selected_protocol, ?flags, "Server confirmed connection");

                (Written::Nothing, VmConnectorState::Handover { selected_protocol })
            }

            VmConnectorState::Handover { .. } => {
                return Err(general_err!(
                    "connector sequence state is already in handover (this is a bug)",
                ));
            }
        };

        self.state = next_state;

        Ok(written)
    }
}

impl VmClientConnector {
    /// Takes over an existing `ClientConnector` and transitions it into a VM-specific connector.
    ///
    /// # Panics
    ///
    /// Panics if the provided `connector` is not in the
    /// [`ClientConnectorState::ConnectionInitiationSendRequest`] state.
    pub fn take_over(connector: ClientConnector) -> ConnectorResult<Self> {
        assert!(
            matches!(connector.state, ClientConnectorState::ConnectionInitiationSendRequest),
            "Invalid connector state for VM connection, expected ConnectionInitiationSendRequest, got: {}",
            connector.state.name()
        );

        debug!("Taking over VM connector");

        let vm_connector_config = VmConnectorConfig::try_from(&connector.config)?;
        let vm_connector = VmClientConnector {
            config: vm_connector_config,
            state: VmConnectorState::EnhancedSecurityUpgrade,
            client_connector: connector,
        };

        Ok(vm_connector)
    }

    pub fn should_hand_over(&self) -> bool {
        matches!(self.state, VmConnectorState::Handover { .. })
    }

    /// Hands the underlying `ClientConnector` back once the VM-specific handshake is done.
    pub fn hand_over(self) -> ConnectorResult<ClientConnector> {
        let VmConnectorState::Handover { selected_protocol } = self.state else {
            return Err(general_err!("Invalid state for handover, expected Handover"));
        };
        let VmClientConnector {
            mut client_connector, ..
        } = self;

        client_connector.state = ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol };

        Ok(client_connector)
    }
}

impl ironrdp_connector::SecurityConnector for VmClientConnector {
    fn should_perform_security_upgrade(&self) -> bool {
        matches!(self.state, VmConnectorState::EnhancedSecurityUpgrade)
    }

    fn mark_security_upgrade_as_done(&mut self) {
        assert!(self.should_perform_security_upgrade());
        self.step(&[], &mut WriteBuf::new()).expect("transition to next state");
        debug_assert!(!self.should_perform_security_upgrade());
    }

    fn should_perform_credssp(&self) -> bool {
        matches!(self.state, VmConnectorState::Credssp)
    }

    fn selected_protocol(&self) -> Option<SecurityProtocol> {
        if self.should_perform_credssp() {
            Some(HYPERV_SECURITY_PROTOCOL)
        } else {
            None
        }
    }

    fn mark_credssp_as_done(&mut self) {
        assert!(self.should_perform_credssp());
        self.step(&[], &mut WriteBuf::new()).expect("transition to next state");
        debug_assert!(!self.should_perform_credssp());
    }

    fn config(&self) -> &ironrdp_connector::Config {
        self.client_connector.config()
    }
}

impl CredsspSequenceFactory for VmClientConnector {
    fn init_credssp(
        &self,
        credentials: ironrdp_connector::Credentials,
        domain: Option<&str>,
        _protocol: SecurityProtocol,
        server_name: ironrdp_connector::ServerName,
        server_public_key: Vec<u8>,
        _kerberos_config: Option<ironrdp_connector::credssp::KerberosConfig>,
    ) -> ConnectorResult<(
        Box<dyn ironrdp_connector::credssp::CredsspSequenceTrait>,
        sspi::credssp::TsRequest,
    )> {
        let credentials = crate::config::Credentials::try_from(&credentials)?;

        let (credssp, ts_request) =
            crate::credssp::VmCredsspSequence::init(credentials, domain, server_name, server_public_key)?;

        let credssp: Box<dyn ironrdp_connector::credssp::CredsspSequenceTrait> = Box::new(credssp);

        Ok((credssp, ts_request))
    }
}
