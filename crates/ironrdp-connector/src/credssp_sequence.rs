use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{nego, PduHint};
use sspi::credssp::{self, ClientState, CredSspClient};
use sspi::generator::{Generator, NetworkRequest};
use sspi::negotiate::ProtocolConfig;
use sspi::Username;

use crate::{
    ClientConnector, ClientConnectorState, ConnectorError, ConnectorErrorKind, ConnectorResult, KerberosConfig,
    ServerName, Written,
};

#[derive(Clone, Copy, Debug)]
struct CredsspTsRequestHint;

const CREDSSP_TS_REQUEST_HINT: CredsspTsRequestHint = CredsspTsRequestHint;

impl PduHint for CredsspTsRequestHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp_pdu::PduResult<Option<usize>> {
        match sspi::credssp::TsRequest::read_length(bytes) {
            Ok(length) => Ok(Some(length)),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(ironrdp_pdu::custom_err!("CredsspTsRequestHint", e)),
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct CredsspEarlyUserAuthResultHint;

const CREDSSP_EARLY_USER_AUTH_RESULT_HINT: CredsspEarlyUserAuthResultHint = CredsspEarlyUserAuthResultHint;

impl PduHint for CredsspEarlyUserAuthResultHint {
    fn find_size(&self, _: &[u8]) -> ironrdp_pdu::PduResult<Option<usize>> {
        Ok(Some(sspi::credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE))
    }
}

pub type CredsspProcessGenerator<'a> = Generator<'a, NetworkRequest, sspi::Result<Vec<u8>>, sspi::Result<ClientState>>;

#[derive(Debug)]
pub struct CredSspSequence {
    client: CredSspClient,
    next_request: Option<credssp::TsRequest>,
    state: CredSSPState,
    selected_protocol: nego::SecurityProtocol,
}

#[derive(Debug, PartialEq)]
pub(crate) enum CredSSPState {
    CredsspInitial,
    CredsspReplyNeeded,
    CredsspEarlyUserAuthResult,
    Finished,
}

impl CredSspSequence {
    pub fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match self.state {
            CredSSPState::CredsspInitial => None,
            CredSSPState::CredsspReplyNeeded => Some(&CREDSSP_TS_REQUEST_HINT),
            CredSSPState::CredsspEarlyUserAuthResult => Some(&CREDSSP_EARLY_USER_AUTH_RESULT_HINT),
            CredSSPState::Finished => None,
        }
    }

    pub fn new(
        connector: &ClientConnector,
        server_name: ServerName,
        server_public_key: Vec<u8>,
        kerberos_config: Option<KerberosConfig>,
    ) -> ConnectorResult<Self> {
        let config = &connector.config;
        if let crate::Credentials::SmartCard { .. } = config.credentials {
            return Err(general_err!(
                "CredSSP with smart card credentials is not currently supported"
            ));
        }

        let credentials = sspi::AuthIdentity {
            username: Username::parse(config.credentials.username()).map_err(|e| custom_err!("parsing username", e))?,
            password: config.credentials.secret().to_owned().into(),
        };

        let server_name = server_name.into_inner();

        let service_principal_name = format!("TERMSRV/{}", &server_name);

        let credssp_config: Box<dyn ProtocolConfig>;
        if let Some(ref krb_config) = kerberos_config {
            credssp_config = Box::new(Into::<sspi::KerberosConfig>::into(krb_config.clone()));
        } else {
            credssp_config = Box::<sspi::ntlm::NtlmConfig>::default();
        }
        info!("using config : {:?}", &credssp_config);

        let client = credssp::CredSspClient::new(
            server_public_key,
            credentials.into(),
            credssp::CredSspMode::WithCredentials,
            credssp::ClientMode::Negotiate(sspi::NegotiateConfig {
                protocol_config: credssp_config,
                package_list: None,
                client_computer_name: server_name,
            }),
            service_principal_name,
        )
        .map_err(|e| ConnectorError::new("CredSSP", ConnectorErrorKind::Credssp(e)))?;

        match connector.state {
            ClientConnectorState::CredSsp { selected_protocol } => Ok(Self {
                client,
                next_request: Some(credssp::TsRequest::default()),
                state: CredSSPState::CredsspInitial,
                selected_protocol,
            }),
            _ => Err(general_err!(
                "Cannot perform cred ssp opeartions when ClientConnector is not in CredSsp state"
            )),
        }
    }

    pub fn is_done(&self) -> bool {
        self.state == CredSSPState::Finished
    }

    pub fn wants_request_from_server(&self) -> bool {
        self.next_request.is_none()
    }

    pub fn read_request_from_server(&mut self, input: &[u8]) -> ConnectorResult<()> {
        match self.state {
            CredSSPState::CredsspInitial | CredSSPState::CredsspReplyNeeded => {
                info!("read request from server: {:?}", input);
                let message = credssp::TsRequest::from_buffer(input)
                    .map_err(|e| reason_err!("CredSSP", "TsRequest decode: {e}"))?;
                debug!(?message, "Received");
                self.next_request = Some(message);
                Ok(())
            }
            CredSSPState::CredsspEarlyUserAuthResult => {
                let early_user_auth_result = credssp::EarlyUserAuthResult::from_buffer(input)
                    .map_err(|e| custom_err!("credssp::EarlyUserAuthResult", e))?;

                debug!(message = ?early_user_auth_result, "Received");

                let credssp::EarlyUserAuthResult::Success = early_user_auth_result else {
                    return Err(ConnectorError::new("CredSSP", ConnectorErrorKind::AccessDenied));
                };
                Ok(())
            }
            _ => Err(general_err!("CredSsp Sequence is Finished")),
        }
    }

    pub fn process(&mut self) -> CredsspProcessGenerator<'_> {
        let request = self.next_request.take().expect("next request");
        info!("Ts request = {:?}", &request);
        self.client.process(request)
    }

    pub fn handle_process_result(&mut self, result: ClientState, output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (size, next_state) = match self.state {
            CredSSPState::CredsspInitial => {
                let (ts_request_from_client, next_state) = match result {
                    ClientState::ReplyNeeded(ts_request) => (ts_request, CredSSPState::CredsspReplyNeeded),
                    ClientState::FinalMessage(ts_request) => (ts_request, CredSSPState::Finished),
                };
                debug!(message = ?ts_request_from_client, "Send");

                let written = write_credssp_request(ts_request_from_client, output)?;
                self.next_request = None;
                Ok((Written::from_size(written)?, next_state))
            }
            CredSSPState::CredsspReplyNeeded => {
                let (ts_request_from_client, next_state) = match result {
                    credssp::ClientState::ReplyNeeded(ts_request) => (ts_request, CredSSPState::CredsspReplyNeeded),
                    credssp::ClientState::FinalMessage(ts_request) => (
                        ts_request,
                        if self.selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX) {
                            CredSSPState::CredsspEarlyUserAuthResult
                        } else {
                            CredSSPState::Finished
                        },
                    ),
                };

                debug!(message = ?ts_request_from_client, "Send");

                let written = write_credssp_request(ts_request_from_client, output)?;
                self.next_request = None;
                Ok((Written::from_size(written)?, next_state))
            }
            CredSSPState::CredsspEarlyUserAuthResult => Ok((Written::Nothing, CredSSPState::Finished)),
            CredSSPState::Finished => Err(general_err!("CredSSP Sequence if finished")),
        }?;
        self.state = next_state;
        Ok(size)
    }
}

fn write_credssp_request(ts_request: credssp::TsRequest, output: &mut WriteBuf) -> ConnectorResult<usize> {
    let length = usize::from(ts_request.buffer_len());

    let unfilled_buffer = output.unfilled_to(length);

    ts_request
        .encode_ts_request(unfilled_buffer)
        .map_err(|e| reason_err!("CredSSP", "TsRequest encode: {e}"))?;

    output.advance(length);

    Ok(length)
}
