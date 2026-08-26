use ironrdp_async::NetworkClient;
use ironrdp_connector::sspi::credssp::{
    CredSspServer, CredentialsProxy, ServerError, ServerMode, ServerState, TsRequest,
};
use ironrdp_connector::sspi::generator::{Generator, GeneratorState};
use ironrdp_connector::sspi::negotiate::ProtocolConfig;
use ironrdp_connector::sspi::{
    self, AuthIdentity, KerberosServerConfig, NegotiateConfig, NetworkRequest, Username, UsernameParts,
};
use ironrdp_connector::{
    ConnectorError, ConnectorErrorKind, ConnectorResult, ServerName, Written, custom_err, general_err,
};
use ironrdp_core::{WriteBuf, other_err};
use ironrdp_pdu::PduHint;
use tracing::debug;

#[derive(Debug)]
pub(crate) enum CredsspState {
    Ongoing,
    Finished(AuthIdentity),
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
        if !usernames_match(&self.credentials.username, username) {
            return Err(std::io::Error::other("invalid username"));
        }

        let mut data = self.credentials.clone();
        // keep the original user/domain
        data.username = username.clone();
        Ok(data)
    }

    fn auth_data(&mut self) -> Result<Vec<Self::AuthenticationData>, std::io::Error> {
        Ok(vec![self.credentials.clone()])
    }
}

fn usernames_match(expected: &Username, requested: &Username) -> bool {
    let requested_parts = requested.parts();
    let requested_account = match requested_parts {
        UsernameParts::UserPrincipalName(parts) => parts.account_name(),
        UsernameParts::DownLevelLogonName(parts) => parts.account_name(),
    };

    match expected.parts() {
        UsernameParts::UserPrincipalName(expected_parts) => {
            let UsernameParts::UserPrincipalName(requested_parts) = requested_parts else {
                return false;
            };
            expected_parts
                .account_name()
                .eq_ignore_ascii_case(requested_parts.account_name())
                && expected_parts.suffix().eq_ignore_ascii_case(requested_parts.suffix())
        }
        UsernameParts::DownLevelLogonName(expected_parts) => {
            if !expected_parts.account_name().eq_ignore_ascii_case(requested_account) {
                return false;
            }

            let Some(expected_domain) = expected_parts.netbios_domain() else {
                return true;
            };
            let UsernameParts::DownLevelLogonName(requested_parts) = requested_parts else {
                return false;
            };
            requested_parts
                .netbios_domain()
                .is_some_and(|requested_domain| expected_domain.eq_ignore_ascii_case(requested_domain))
        }
    }
}

pub(crate) async fn resolve_generator(
    generator: &mut CredsspProcessGenerator<'_>,
    network_client: &mut impl NetworkClient,
) -> Result<ServerState, ServerError> {
    let mut state = match std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| generator.start())) {
        Ok(s) => s,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .copied()
                .map(String::from)
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "CredSSP processing panic (generator.start)".to_owned());
            return Err(ServerError {
                ts_request: None,
                error: sspi::Error::new(sspi::ErrorKind::InternalError, msg),
            });
        }
    };

    loop {
        match state {
            GeneratorState::Suspended(request) => {
                let response = network_client.send(&request).await.map_err(|err| ServerError {
                    ts_request: None,
                    error: sspi::Error::new(sspi::ErrorKind::InternalError, err),
                })?;
                state = match std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| generator.resume(Ok(response))))
                {
                    Ok(s) => s,
                    Err(panic) => {
                        let msg = panic
                            .downcast_ref::<&str>()
                            .copied()
                            .map(String::from)
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "CredSSP processing panic".to_owned());
                        return Err(ServerError {
                            ts_request: None,
                            error: sspi::Error::new(sspi::ErrorKind::InternalError, msg),
                        });
                    }
                };
            }
            GeneratorState::Completed(client_state) => break client_state,
        }
    }
}

impl<'a> CredsspSequence<'a> {
    pub fn next_pdu_hint(&self) -> ConnectorResult<Option<&dyn PduHint>> {
        match &self.state {
            CredsspState::Ongoing => Ok(Some(&CREDSSP_TS_REQUEST_HINT)),
            CredsspState::Finished(_) => Ok(None),
            CredsspState::ServerError(err) => Err(custom_err!("Credssp server error", err.clone())),
        }
    }

    /// Consume the completed CredSSP sequence and return its captured identity.
    pub fn into_identity(self) -> Option<AuthIdentity> {
        match self.state {
            CredsspState::Finished(identity) => Some(identity),
            CredsspState::Ongoing | CredsspState::ServerError(_) => None,
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

        let server_mode = if let Some(krb_config) = krb_config {
            let credssp_config: Box<dyn ProtocolConfig> = Box::new(krb_config);
            ServerMode::Negotiate(NegotiateConfig {
                protocol_config: credssp_config,
                package_list: None,
                client_computer_name,
            })
        } else {
            let credssp_config: Box<dyn ProtocolConfig> = Box::new(sspi::ntlm::NtlmConfig::default());
            ServerMode::Negotiate(NegotiateConfig {
                protocol_config: credssp_config,
                // Restrict to NTLM when no Kerberos server config is provided.
                // This avoids environment-dependent Kerberos negotiation.
                package_list: Some("!kerberos,!pku2u".to_owned()),
                client_computer_name,
            })
        };

        let server = CredSspServer::new(public_key, credentials, server_mode)
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
                let decode = || -> ConnectorResult<Option<TsRequest>> {
                    let message = TsRequest::from_buffer(input).map_err(|e| custom_err!("TsRequest", e))?;

                    debug!(?message, "Received");
                    Ok(Some(message))
                };

                match std::panic::catch_unwind(core::panic::AssertUnwindSafe(decode)) {
                    Ok(res) => res,
                    Err(panic) => {
                        let msg = panic
                            .downcast_ref::<&str>()
                            .copied()
                            .map(String::from)
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "CredSSP decode_client_message panic".to_owned());
                        Err(ConnectorError::new(
                            "CredSSP decode",
                            ConnectorErrorKind::Credssp(sspi::Error::new(sspi::ErrorKind::InternalError, msg)),
                        ))
                    }
                }
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
            Ok(ServerState::Finished(identity)) => (None, CredsspState::Finished(identity)),
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
