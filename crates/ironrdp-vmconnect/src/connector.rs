use core::mem;

use ironrdp_async::{Framed, FramedRead, FramedWrite, PcbSent, single_sequence_step};
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectorError, ConnectorErrorExt as _, ConnectorErrorKind, ConnectorResult,
    NegotiationFailure, SecurityConnector, Sequence, State, Written, general_err, reason_err,
};
use ironrdp_core::{WriteBuf, decode};
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::{PduHint, nego};
use tracing::{debug, error, info};

/// Protocols advertised in the X.224 connection request of a vmconnect session.
const HYPERV_SECURITY_PROTOCOL: SecurityProtocol = SecurityProtocol::HYBRID_EX
    .union(SecurityProtocol::SSL)
    .union(SecurityProtocol::HYBRID);

/// CredSSP semantics used by vmconnect.
///
/// CredSSP runs before X.224 negotiation, so no protocol has been selected yet; the host expects
/// plain HYBRID-style CredSSP (no Early User Authorization Result).
const VMCONNECT_CREDSSP_PROTOCOL: SecurityProtocol = SecurityProtocol::HYBRID;

#[derive(Default, Debug)]
enum VmConnectorState {
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
            Self::EnhancedSecurityUpgrade => "EnhancedSecurityUpgrade",
            Self::Credssp => "Credssp",
            Self::ConnectionInitiationSendRequest => "ConnectionInitiationSendRequest",
            Self::ConnectionInitiationWaitConfirm => "ConnectionInitiationWaitConfirm",
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

/// Drives the vmconnect pre-connection sequence, wrapping the standard [`ClientConnector`].
///
/// The wrapped connector is held untouched until [`hand_over`](Self::hand_over), which returns it
/// positioned at the Basic Settings Exchange with the protocol negotiated here.
#[derive(Debug)]
pub struct VmClientConnector {
    state: VmConnectorState,
    client_connector: ClientConnector,
}

impl Sequence for VmClientConnector {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match &self.state {
            VmConnectorState::ConnectionInitiationWaitConfirm => Some(&ironrdp_pdu::X224_HINT),
            _ => None,
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            VmConnectorState::ConnectionInitiationSendRequest => {
                debug!("Connection Initiation");

                let config = &self.client_connector.config;
                let ironrdp_connector::Credentials::UsernamePassword { username, .. } = &config.credentials else {
                    return Err(general_err!("vmconnect requires username/password credentials"));
                };

                let connection_request = nego::ConnectionRequest {
                    nego_data: config
                        .request_data
                        .clone()
                        .or_else(|| Some(nego::NegoRequestData::cookie(username.clone()))),
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
                        return Err(ConnectorError::new(
                            "negotiation failure",
                            ConnectorErrorKind::Negotiation(NegotiationFailure::from(code)),
                        ));
                    }
                };

                info!(?selected_protocol, ?flags, "Server confirmed connection");

                // Direct Approach ran plain HYBRID CredSSP before this exchange, so the host must
                // select exactly HYBRID here (MS-RDPBCGR 5.4.2.2). Anything else — SSL, or HYBRID_EX
                // which would imply an Early User Authorization Result we never negotiated — would
                // be echoed into Client Core Data as a protocol we did not actually perform.
                if selected_protocol != VMCONNECT_CREDSSP_PROTOCOL {
                    return Err(reason_err!(
                        "Initiation",
                        "vmconnect requires the server to select {VMCONNECT_CREDSSP_PROTOCOL}, but it selected {selected_protocol}",
                    ));
                }

                (Written::Nothing, VmConnectorState::Handover { selected_protocol })
            }
            invalid => {
                return Err(reason_err!(
                    "VmConnect",
                    "invalid connector state for step: {}",
                    invalid.name()
                ));
            }
        };

        self.state = next_state;

        Ok(written)
    }
}

impl VmClientConnector {
    /// Takes over a fresh [`ClientConnector`] to run the vmconnect pre-connection sequence.
    ///
    /// Requires proof that the preconnection blob was already sent ([`PcbSent`]), and that the
    /// wrapped connector has not started its own connection initiation yet.
    pub fn take_over(_: PcbSent, connector: ClientConnector) -> ConnectorResult<Self> {
        if !matches!(connector.state, ClientConnectorState::ConnectionInitiationSendRequest) {
            return Err(general_err!(
                "invalid connector state for VM connection, expected ConnectionInitiationSendRequest"
            ));
        }

        debug!("Taking over VM connector");

        if !matches!(
            connector.config.credentials,
            ironrdp_connector::Credentials::UsernamePassword { .. }
        ) {
            return Err(general_err!("vmconnect requires username/password credentials"));
        }

        Ok(VmClientConnector {
            state: VmConnectorState::EnhancedSecurityUpgrade,
            client_connector: connector,
        })
    }

    pub(crate) fn should_hand_over(&self) -> bool {
        matches!(self.state, VmConnectorState::Handover { .. })
    }

    /// Hands the wrapped [`ClientConnector`] back once the vmconnect sequence is done.
    pub(crate) fn hand_over(self) -> ConnectorResult<ClientConnector> {
        let VmConnectorState::Handover { selected_protocol } = self.state else {
            return Err(general_err!("invalid state for handover, expected Handover"));
        };

        let mut client_connector = self.client_connector;
        client_connector.state = ClientConnectorState::BasicSettingsExchangeSendInitial { selected_protocol };

        Ok(client_connector)
    }
}

impl SecurityConnector for VmClientConnector {
    fn should_perform_security_upgrade(&self) -> bool {
        matches!(self.state, VmConnectorState::EnhancedSecurityUpgrade)
    }

    fn mark_security_upgrade_as_done(&mut self) {
        assert!(self.should_perform_security_upgrade());
        self.state = VmConnectorState::Credssp;
    }

    fn should_perform_credssp(&self) -> bool {
        matches!(self.state, VmConnectorState::Credssp)
    }

    fn credssp_protocol(&self) -> Option<SecurityProtocol> {
        self.should_perform_credssp().then_some(VMCONNECT_CREDSSP_PROTOCOL)
    }

    fn mark_credssp_as_done(&mut self) {
        assert!(self.should_perform_credssp());
        self.state = VmConnectorState::ConnectionInitiationSendRequest;
    }

    fn config(&self) -> &ironrdp_connector::Config {
        &self.client_connector.config
    }
}

/// Runs the vmconnect X.224 negotiation to completion and hands the session back to the standard
/// [`ClientConnector`], ready for [`connect_finalize`](ironrdp_async::connect_finalize).
pub async fn run_until_handover<S>(
    framed: &mut Framed<S>,
    mut connector: VmClientConnector,
) -> ConnectorResult<ClientConnector>
where
    S: FramedRead + FramedWrite,
{
    let mut buf = WriteBuf::new();

    let connector = loop {
        single_sequence_step(framed, &mut connector, &mut buf).await?;

        if connector.should_hand_over() {
            break connector.hand_over()?;
        }
    };

    info!("Handover to client connector");

    Ok(connector)
}
