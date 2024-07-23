use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{nego, PduHint};
use sspi::credssp::{self, ClientState, CredSspClient};
use sspi::generator::{Generator, NetworkRequest};
use sspi::negotiate::ProtocolConfig;
use sspi::Username;

use crate::{
    ClientConnector, ClientConnectorState, ConnectorError, ConnectorErrorKind, ConnectorResult, ServerName, Written,
};

#[derive(Debug, Clone, Default)]
pub struct KerberosConfig {
    pub kdc_proxy_url: Option<url::Url>,
    pub hostname: Option<String>,
}

impl KerberosConfig {
    pub fn new(kdc_proxy_url: Option<String>, hostname: Option<String>) -> ConnectorResult<Self> {
        let kdc_proxy_url = kdc_proxy_url
            .map(|url| url::Url::parse(&url))
            .transpose()
            .map_err(|e| custom_err!("invalid KDC URL", e))?;
        Ok(Self {
            kdc_proxy_url,
            hostname,
        })
    }
}

impl From<KerberosConfig> for sspi::KerberosConfig {
    fn from(val: KerberosConfig) -> Self {
        sspi::KerberosConfig {
            kdc_url: val.kdc_proxy_url,
            client_computer_name: val.hostname,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CredsspTsRequestHint;

const CREDSSP_TS_REQUEST_HINT: CredsspTsRequestHint = CredsspTsRequestHint;

impl PduHint for CredsspTsRequestHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp_pdu::PduResult<Option<usize>> {
        match credssp::TsRequest::read_length(bytes) {
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
        Ok(Some(credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE))
    }
}

pub type CredsspProcessGenerator<'a> = Generator<'a, NetworkRequest, sspi::Result<Vec<u8>>, sspi::Result<ClientState>>;

#[derive(Debug)]
pub struct CredsspSequence {
    client: CredSspClient,
    state: CredsspState,
    selected_protocol: nego::SecurityProtocol,
}

#[derive(Debug, PartialEq)]
pub(crate) enum CredsspState {
    Ongoing,
    EarlyUserAuthResult,
    Finished,
}

impl CredsspSequence {
    pub fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match self.state {
            CredsspState::Ongoing => Some(&CREDSSP_TS_REQUEST_HINT),
            CredsspState::EarlyUserAuthResult => Some(&CREDSSP_EARLY_USER_AUTH_RESULT_HINT),
            CredsspState::Finished => None,
        }
    }

    /// `server_name` must be the actual target server hostname (as opposed to the proxy)
    pub fn init(
        connector: &ClientConnector,
        server_name: ServerName,
        server_public_key: Vec<u8>,
        kerberos_config: Option<KerberosConfig>,
    ) -> ConnectorResult<(Self, credssp::TsRequest)> {
        let config = &connector.config;
        if let crate::Credentials::SmartCard { .. } = config.credentials {
            return Err(general_err!(
                "CredSSP with smart card credentials is not currently supported"
            ));
        }

        let username = Username::new(config.credentials.username(), config.domain.as_deref())
            .map_err(|e| custom_err!("invalid username", e))?;

        let credentials = sspi::AuthIdentity {
            username,
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
        debug!(?credssp_config);

        let client = CredSspClient::new(
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
            ClientConnectorState::Credssp { selected_protocol } => {
                let sequence = Self {
                    client,
                    state: CredsspState::Ongoing,
                    selected_protocol,
                };

                let initial_request = credssp::TsRequest::default();

                Ok((sequence, initial_request))
            }
            _ => Err(general_err!("invalid connector state for CredSSP sequence")),
        }
    }

    /// Returns Some(ts_request) when a TS request is received from server,
    /// and None when an early user auth result PDU is received instead.
    pub fn decode_server_message(&mut self, input: &[u8]) -> ConnectorResult<Option<credssp::TsRequest>> {
        match self.state {
            CredsspState::Ongoing => {
                let message = credssp::TsRequest::from_buffer(input).map_err(|e| custom_err!("TsRequest", e))?;
                debug!(?message, "Received");
                Ok(Some(message))
            }
            CredsspState::EarlyUserAuthResult => {
                let early_user_auth_result = credssp::EarlyUserAuthResult::from_buffer(input)
                    .map_err(|e| custom_err!("EarlyUserAuthResult", e))?;

                debug!(message = ?early_user_auth_result, "Received");

                match early_user_auth_result {
                    credssp::EarlyUserAuthResult::Success => {
                        self.state = CredsspState::Finished;
                        Ok(None)
                    }
                    credssp::EarlyUserAuthResult::AccessDenied => {
                        Err(ConnectorError::new("CredSSP", ConnectorErrorKind::AccessDenied))
                    }
                }
            }
            _ => Err(general_err!(
                "attempted to feed server request to CredSSP sequence in an unexpected state"
            )),
        }
    }

    pub fn process_ts_request(&mut self, request: credssp::TsRequest) -> CredsspProcessGenerator<'_> {
        self.client.process(request)
    }

    pub fn handle_process_result(&mut self, result: ClientState, output: &mut WriteBuf) -> ConnectorResult<Written> {
        let (size, next_state) = match self.state {
            CredsspState::Ongoing => {
                let (ts_request_from_client, next_state) = match result {
                    ClientState::ReplyNeeded(ts_request) => (ts_request, CredsspState::Ongoing),
                    ClientState::FinalMessage(ts_request) => (
                        ts_request,
                        if self.selected_protocol.contains(nego::SecurityProtocol::HYBRID_EX) {
                            CredsspState::EarlyUserAuthResult
                        } else {
                            CredsspState::Finished
                        },
                    ),
                };

                debug!(message = ?ts_request_from_client, "Send");

                let written = write_credssp_request(ts_request_from_client, output)?;

                Ok((Written::from_size(written)?, next_state))
            }
            CredsspState::EarlyUserAuthResult => Ok((Written::Nothing, CredsspState::Finished)),
            CredsspState::Finished => Err(general_err!("CredSSP sequence is already done")),
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
        .map_err(|e| custom_err!("TsRequest", e))?;

    output.advance(length);

    Ok(length)
}
