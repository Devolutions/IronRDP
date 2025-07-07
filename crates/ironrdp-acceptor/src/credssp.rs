use ironrdp_async::AsyncNetworkClient;
use ironrdp_connector::sspi::credssp::{
    CredSspServer, CredentialsProxy, ServerError, ServerMode, ServerState, TsRequest,
};
use ironrdp_connector::sspi::generator::{Generator, GeneratorState};
use ironrdp_connector::sspi::negotiate::ProtocolConfig;
use ironrdp_connector::sspi::{self, AuthIdentity, KerberosServerConfig, NegotiateConfig, NetworkRequest, Username};
use ironrdp_connector::{
    custom_err, general_err, ConnectorError, ConnectorErrorKind, ConnectorResult, ServerName, Written,
};
use ironrdp_core::{other_err, WriteBuf};
use ironrdp_pdu::PduHint;

#[derive(Debug)]
pub(crate) enum CredsspState {
    Ongoing,
    Finished,
    ServerError(sspi::Error),
}

#[derive(Clone, Copy, Debug)]
struct CredsspTsRequestHint;

const CREDSSP_TS_REQUEST_HINT: CredsspTsRequestHint = CredsspTsRequestHint;

impl PduHint for CredsspTsRequestHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp_core::DecodeResult<Option<(bool, usize)>> {
        match TsRequest::read_length(bytes) {
            Ok(length) => Ok(Some((true, length))),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(other_err!("CredsspTsRequestHint", source: e)),
        }
    }
}

pub type CredsspProcessGenerator<'a> =
    Generator<'a, NetworkRequest, sspi::Result<Vec<u8>>, Result<ServerState, ServerError>>;

#[derive(Debug)]
pub struct CredsspSequence<'a> {
    server: CredSspServer<CredentialsProxyImpl<'a>>,
    state: CredsspState,
}

#[derive(Debug)]
struct CredentialsProxyImpl<'a> {
    credentials: &'a AuthIdentity,
}

impl<'a> CredentialsProxyImpl<'a> {
    fn new(credentials: &'a AuthIdentity) -> Self {
        Self { credentials }
    }
}

impl CredentialsProxy for CredentialsProxyImpl<'_> {
    type AuthenticationData = AuthIdentity;

    fn auth_data_by_user(&mut self, username: &Username) -> std::io::Result<Self::AuthenticationData> {
        if username.account_name() != self.credentials.username.account_name() {
            return Err(std::io::Error::other("invalid username"));
        }

        let mut data = self.credentials.clone();
        // keep the original user/domain
        data.username = username.clone();
        Ok(data)
    }
}

pub(crate) async fn resolve_generator(
    generator: &mut CredsspProcessGenerator<'_>,
    network_client: &mut dyn AsyncNetworkClient,
) -> Result<ServerState, ServerError> {
    let mut state = generator.start();

    loop {
        match state {
            GeneratorState::Suspended(request) => {
                let response = network_client
                    .send(&request)
                    .await
                    .inspect_err(|err| error!(?err, "Failed to send a Kerberos message"))
                    .map_err(|err| ServerError {
                        ts_request: None,
                        error: sspi::Error::new(sspi::ErrorKind::InternalError, err),
                    })?;
                state = generator.resume(Ok(response));
            }
            GeneratorState::Completed(client_state) => break client_state,
        }
    }
}

impl<'a> CredsspSequence<'a> {
    pub fn next_pdu_hint(&self) -> ConnectorResult<Option<&dyn PduHint>> {
        match &self.state {
            CredsspState::Ongoing => Ok(Some(&CREDSSP_TS_REQUEST_HINT)),
            CredsspState::Finished => Ok(None),
            CredsspState::ServerError(err) => Err(custom_err!("Credssp server error", err.clone())),
        }
    }

    pub fn init(
        creds: &'a AuthIdentity,
        client_computer_name: ServerName,
        public_key: Vec<u8>,
        krb_config: Option<KerberosServerConfig>,
    ) -> ConnectorResult<Self> {
        let client_computer_name = client_computer_name.into_inner();
        let credentials = CredentialsProxyImpl::new(creds);

        let credssp_config: Box<dyn ProtocolConfig> = if let Some(krb_config) = krb_config {
            Box::new(krb_config)
        } else {
            Box::<sspi::ntlm::NtlmConfig>::default()
        };

        let server = CredSspServer::new(
            public_key,
            credentials,
            ServerMode::Negotiate(NegotiateConfig {
                protocol_config: credssp_config,
                package_list: None,
                client_computer_name,
            }),
        )
        .map_err(|e| ConnectorError::new("CredSSP", ConnectorErrorKind::Credssp(e)))?;

        let sequence = Self {
            server,
            state: CredsspState::Ongoing,
        };

        Ok(sequence)
    }

    /// Returns Some(ts_request) when a TS request is received from client,
    pub fn decode_client_message(&mut self, input: &[u8]) -> ConnectorResult<Option<TsRequest>> {
        match self.state {
            CredsspState::Ongoing => {
                let message = TsRequest::from_buffer(input).map_err(|e| custom_err!("TsRequest", e))?;
                debug!(?message, "Received");
                Ok(Some(message))
            }
            _ => Err(general_err!(
                "attempted to feed client request to CredSSP sequence in an unexpected state"
            )),
        }
    }

    pub fn process_ts_request(&mut self, request: TsRequest) -> CredsspProcessGenerator<'_> {
        self.server.process(request)
    }

    pub fn handle_process_result(
        &mut self,
        result: Result<ServerState, ServerError>,
        output: &mut WriteBuf,
    ) -> ConnectorResult<Written> {
        let (ts_request, next_state) = match result {
            Ok(ServerState::ReplyNeeded(ts_request)) => (Some(ts_request), CredsspState::Ongoing),
            Ok(ServerState::Finished(_id)) => (None, CredsspState::Finished),
            Err(err) => (
                err.ts_request.map(|ts_request| *ts_request),
                CredsspState::ServerError(err.error),
            ),
        };

        self.state = next_state;
        if let Some(ts_request) = ts_request {
            debug!(?ts_request, "Send");
            let length = usize::from(ts_request.buffer_len());
            let unfilled_buffer = output.unfilled_to(length);

            ts_request
                .encode_ts_request(unfilled_buffer)
                .map_err(|e| custom_err!("TsRequest", e))?;

            output.advance(length);

            Ok(Written::from_size(length)?)
        } else {
            Ok(Written::Nothing)
        }
    }
}
