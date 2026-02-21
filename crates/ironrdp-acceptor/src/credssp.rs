use ironrdp_async::NetworkClient;
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
use picky_asn1::wrapper::{ExplicitContextTag0, ExplicitContextTag1, ExplicitContextTag2, OctetStringAsn1, Optional};
use picky_asn1_der::Asn1RawDer;
use picky_krb::constants::gss_api::{ACCEPT_COMPLETE, ACCEPT_INCOMPLETE};
use picky_krb::gss_api::{ApplicationTag0, GssApiNegInit, NegTokenTarg, NegTokenTarg1};
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
    spnego_wrapped: bool,
}

fn try_unwrap_spnego(token: &[u8]) -> Option<Vec<u8>> {
    // `picky_krb`'s `ApplicationTag0` deserializer uses an internal `unwrap()` and may panic when
    // fed non-GSSAPI bytes (e.g., raw NTLM tokens). Treat panics as a non-match.
    let init = std::panic::catch_unwind(|| picky_asn1_der::from_bytes::<ApplicationTag0<GssApiNegInit>>(token));
    if let Ok(Ok(init)) = init {
        let mech_token = init.0.neg_token_init.0.mech_token.0?;
        return Some(mech_token.0 .0);
    }

    let targ = std::panic::catch_unwind(|| picky_asn1_der::from_bytes::<NegTokenTarg1>(token));
    if let Ok(Ok(targ)) = targ {
        let response_token = targ.0.response_token.0?;
        return Some(response_token.0 .0);
    }

    None
}

fn wrap_spnego_ntlm_reply(raw_token: Vec<u8>) -> std::io::Result<Vec<u8>> {
    let neg_result = if raw_token.is_empty() {
        ACCEPT_COMPLETE
    } else {
        ACCEPT_INCOMPLETE
    };

    let response_token = if raw_token.is_empty() {
        Optional::from(None)
    } else {
        Optional::from(Some(ExplicitContextTag2::from(OctetStringAsn1::from(raw_token))))
    };

    let targ = ExplicitContextTag1::from(NegTokenTarg {
        neg_result: Optional::from(Some(ExplicitContextTag0::from(Asn1RawDer(neg_result.to_vec())))),
        supported_mech: Optional::from(None),
        response_token,
        mech_list_mic: Optional::from(None),
    });

    picky_asn1_der::to_vec(&targ).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
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
    network_client: &mut impl NetworkClient,
) -> Result<ServerState, ServerError> {
    let mut state = generator.start();

    loop {
        match state {
            GeneratorState::Suspended(request) => {
                let response = network_client.send(&request).await.map_err(|err| ServerError {
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
            CredsspState::Finished(_) => Ok(None),
            CredsspState::ServerError(err) => Err(custom_err!("Credssp server error", err.clone())),
        }
    }

    /// Take the [`AuthIdentity`] captured from the completed CredSSP handshake.
    ///
    /// Returns `Some` exactly once after the sequence finishes successfully.
    /// Subsequent calls return `None`.
    pub fn take_identity(&mut self) -> Option<AuthIdentity> {
        if let CredsspState::Finished(identity) = &self.state {
            let identity = identity.clone();
            // Replace with a sentinel that signals "already taken".
            self.state = CredsspState::Ongoing; // won't be re-entered; caller already called mark_credssp_as_done
            Some(identity)
        } else {
            None
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

        // NOTE: we default to NTLM when no explicit Kerberos config is provided.
        // Using the Negotiate/Kerberos path can trigger picky-krb panics on some
        // environments (observed on IT-HELP-TEST). This still enables CredSSP/NLA
        // for typical test setups while we iterate on full SSPI integration.
        let server_mode = if let Some(krb_config) = krb_config {
            let credssp_config: Box<dyn ProtocolConfig> = Box::new(krb_config);
            ServerMode::Negotiate(NegotiateConfig {
                protocol_config: credssp_config,
                package_list: None,
                client_computer_name,
            })
        } else {
            ServerMode::Ntlm(sspi::ntlm::NtlmConfig::default())
        };

        let server = CredSspServer::new(public_key, credentials, server_mode)
            .map_err(|e| ConnectorError::new("CredSSP", ConnectorErrorKind::Credssp(e)))?;

        let sequence = Self {
            server,
            state: CredsspState::Ongoing,
            spnego_wrapped: false,
        };

        Ok(sequence)
    }

    /// Returns Some(ts_request) when a TS request is received from client,
    pub fn decode_client_message(&mut self, input: &[u8]) -> ConnectorResult<Option<TsRequest>> {
        match self.state {
            CredsspState::Ongoing => {
                let mut message = TsRequest::from_buffer(input).map_err(|e| custom_err!("TsRequest", e))?;

                if let Some(nego_tokens) = message.nego_tokens.take() {
                    if let Some(inner) = try_unwrap_spnego(&nego_tokens) {
                        self.spnego_wrapped = true;
                        message.nego_tokens = Some(inner);
                    } else {
                        message.nego_tokens = Some(nego_tokens);
                    }
                }

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
            Ok(ServerState::Finished(identity)) => (None, CredsspState::Finished(identity)),
            Err(err) => (
                err.ts_request.map(|ts_request| *ts_request),
                CredsspState::ServerError(err.error),
            ),
        };

        self.state = next_state;
        if let Some(ts_request) = ts_request {
            let mut ts_request = ts_request;

            if self.spnego_wrapped {
                if let Some(nego_tokens) = ts_request.nego_tokens.take() {
                    let wrapped = wrap_spnego_ntlm_reply(nego_tokens).map_err(|e| custom_err!("SPNEGO", e))?;
                    ts_request.nego_tokens = Some(wrapped);
                }
            }

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
