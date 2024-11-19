use ironrdp_connector::credssp::KerberosConfig;
use ironrdp_connector::sspi::credssp::{
    ClientMode, CredSspServer, CredentialsProxy, ServerError, ServerState, TsRequest,
};
use ironrdp_connector::sspi::negotiate::ProtocolConfig;
use ironrdp_connector::sspi::{self, AuthIdentity, Username};
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

#[derive(Debug)]
pub(crate) struct CredsspSequence<'a> {
    server: CredSspServer<CredentialsProxyImpl<'a>>,
    state: CredsspState,
    // selected_protocol: nego::SecurityProtocol,
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

impl<'a> CredsspSequence<'a> {
    pub(crate) fn next_pdu_hint(&self) -> ConnectorResult<Option<&dyn PduHint>> {
        match &self.state {
            CredsspState::Ongoing => Ok(Some(&CREDSSP_TS_REQUEST_HINT)),
            CredsspState::Finished => Ok(None),
            CredsspState::ServerError(err) => Err(custom_err!("Credssp server error", err.clone())),
        }
    }

    pub(crate) fn init(
        creds: &'a AuthIdentity,
        client_computer_name: ServerName,
        public_key: Vec<u8>,
        kerberos_config: Option<KerberosConfig>,
    ) -> ConnectorResult<Self> {
        let client_computer_name = client_computer_name.into_inner();
        let credentials = CredentialsProxyImpl::new(creds);
        let credssp_config: Box<dyn ProtocolConfig>;
        if let Some(ref krb_config) = kerberos_config {
            credssp_config = Box::new(Into::<sspi::KerberosConfig>::into(krb_config.clone()));
        } else {
            credssp_config = Box::<sspi::ntlm::NtlmConfig>::default();
        }

        debug!(?credssp_config);
        let server = CredSspServer::new(
            public_key,
            credentials,
            ClientMode::Negotiate(sspi::NegotiateConfig {
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
    pub(crate) fn decode_client_message(&mut self, input: &[u8]) -> ConnectorResult<Option<TsRequest>> {
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

    pub(crate) fn process_ts_request(&mut self, request: TsRequest) -> Result<ServerState, Box<ServerError>> {
        Ok(self.server.process(request)?)
    }

    pub(crate) fn handle_process_result(
        &mut self,
        result: Result<ServerState, Box<ServerError>>,
        output: &mut WriteBuf,
    ) -> ConnectorResult<Written> {
        let (ts_request, next_state) = match result {
            Ok(ServerState::ReplyNeeded(ts_request)) => (Some(ts_request), CredsspState::Ongoing),
            Ok(ServerState::Finished(_id)) => (None, CredsspState::Finished),
            Err(err) => (Some(err.ts_request), CredsspState::ServerError(err.error)),
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
