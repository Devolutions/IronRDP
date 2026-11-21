use ironrdp_core::{WriteBuf, other_err};
use ironrdp_pdu::{PduHint, nego};
use picky::key::PrivateKey;
use picky_asn1_x509::{Certificate, ExtensionView, GeneralName, oids};
use sspi::credssp::{self, ClientState, CredSspClient};
use sspi::generator::{Generator, NetworkRequest};
use sspi::{Secret, Username};
use tracing::debug;

use crate::{
    ConnectorError, ConnectorErrorKind, ConnectorResult, Credentials, ServerName, Written, custom_err, general_err,
};

/// Strategy for resolving the KDC to use for Kerberos authentication.
#[derive(Debug, Clone)]
pub enum KdcResolution {
    /// Use IAKerb extension to proxy KDC communication through the server to the LocalKDC.
    IAKerb,
    /// External KDC URL.
    KdcUrl(Option<url::Url>),
}

impl From<KdcResolution> for sspi::KdcResolution {
    fn from(val: KdcResolution) -> Self {
        match val {
            KdcResolution::IAKerb => sspi::KdcResolution::IAKerb,
            KdcResolution::KdcUrl(url) => sspi::KdcResolution::KdcUrl(url),
        }
    }
}

#[derive(Debug, Clone)]
pub struct KerberosConfig {
    pub kdc_resolution: KdcResolution,
    pub hostname: String,
}

impl KerberosConfig {
    pub fn new_with_kdc_url(kdc_proxy_url: Option<String>, hostname: String) -> ConnectorResult<Self> {
        let kdc_proxy_url = kdc_proxy_url
            .map(|url| url::Url::parse(&url))
            .transpose()
            .map_err(|e| custom_err!("invalid KDC URL", e))?;
        Ok(Self {
            kdc_resolution: KdcResolution::KdcUrl(kdc_proxy_url),
            hostname,
        })
    }

    pub fn new_with_iakerb(hostname: String) -> Self {
        Self {
            kdc_resolution: KdcResolution::IAKerb,
            hostname,
        }
    }
}

/// The security package CredSSP authenticates with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredsspPackage {
    /// NTLM only.
    Ntlm,
    /// SPNEGO, preferring Kerberos and allowing NTLM.
    ///
    /// Without a [`KerberosConfig`], Kerberos is used when sspi finds a KDC for the user's realm, and NTLM otherwise.
    /// With one, Kerberos starts right away; sspi falls back to NTLM only for some errors, and in the CredSSP exchange
    /// a KDC that cannot be found can surface after that point, so the exchange fails instead.
    Negotiate,
    /// SPNEGO restricted to Kerberos: NTLM is never negotiated or used as a fallback.
    Kerberos,
}

impl From<KerberosConfig> for sspi::KerberosConfig {
    fn from(val: KerberosConfig) -> Self {
        sspi::KerberosConfig {
            kdc_resolution: val.kdc_resolution.into(),
            client_computer_name: val.hostname,
        }
    }
}

fn package_for(kerberos_config: Option<&KerberosConfig>) -> CredsspPackage {
    if kerberos_config.is_some() {
        CredsspPackage::Negotiate
    } else {
        CredsspPackage::Ntlm
    }
}

#[derive(Clone, Copy, Debug)]
struct CredsspTsRequestHint;

const CREDSSP_TS_REQUEST_HINT: CredsspTsRequestHint = CredsspTsRequestHint;

impl PduHint for CredsspTsRequestHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp_core::DecodeResult<Option<(bool, usize)>> {
        match credssp::TsRequest::read_length(bytes) {
            Ok(length) => Ok(Some((true, length))),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(other_err!("CredsspTsRequestHint", source: e)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CredsspEarlyUserAuthResultHint;

const CREDSSP_EARLY_USER_AUTH_RESULT_HINT: CredsspEarlyUserAuthResultHint = CredsspEarlyUserAuthResultHint;

impl PduHint for CredsspEarlyUserAuthResultHint {
    fn find_size(&self, _: &[u8]) -> ironrdp_core::DecodeResult<Option<(bool, usize)>> {
        Ok(Some((true, credssp::EARLY_USER_AUTH_RESULT_PDU_SIZE)))
    }
}

pub type CredsspProcessGenerator<'a> = Generator<'a, NetworkRequest, sspi::Result<Vec<u8>>, sspi::Result<ClientState>>;

#[derive(Debug)]
pub struct CredsspSequence {
    client: CredSspClient,
    state: CredsspState,
    selected_protocol: nego::SecurityProtocol,
    remote_guard: bool,
    #[cfg(feature = "remote-guard")]
    remote_guard_ntlm: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) enum CredsspState {
    Ongoing,
    EarlyUserAuthResult,
    Finished,
    Failed,
}

impl CredsspSequence {
    pub fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
        match self.state {
            CredsspState::Ongoing => Some(&CREDSSP_TS_REQUEST_HINT),
            CredsspState::EarlyUserAuthResult => Some(&CREDSSP_EARLY_USER_AUTH_RESULT_HINT),
            CredsspState::Finished | CredsspState::Failed => None,
        }
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.state == CredsspState::Finished
    }

    /// `server_name` must be the actual target server hostname (as opposed to the proxy)
    ///
    /// Uses [`CredsspPackage::Negotiate`] when `kerberos_config` is set, and [`CredsspPackage::Ntlm`] otherwise.
    pub fn init(
        credentials: Credentials,
        domain: Option<&str>,
        protocol: nego::SecurityProtocol,
        server_name: ServerName,
        server_public_key: Vec<u8>,
        kerberos_config: Option<KerberosConfig>,
    ) -> ConnectorResult<(Self, credssp::TsRequest)> {
        let package = package_for(kerberos_config.as_ref());
        Self::init_with_package(
            credentials,
            domain,
            protocol,
            server_name,
            server_public_key,
            package,
            kerberos_config,
        )
    }

    /// Like [`CredsspSequence::init`], with the security package chosen independently of the Kerberos
    /// configuration. `kerberos_config` is optional for [`CredsspPackage::Negotiate`] and
    /// [`CredsspPackage::Kerberos`], and not allowed for [`CredsspPackage::Ntlm`].
    pub fn init_with_package(
        credentials: Credentials,
        domain: Option<&str>,
        protocol: nego::SecurityProtocol,
        server_name: ServerName,
        server_public_key: Vec<u8>,
        package: CredsspPackage,
        kerberos_config: Option<KerberosConfig>,
    ) -> ConnectorResult<(Self, credssp::TsRequest)> {
        Self::init_with_mode(
            credentials,
            domain,
            protocol,
            server_name,
            server_public_key,
            package,
            kerberos_config,
            false,
        )
    }

    /// Constructs the authentication exchange using the connector's negotiated security policy.
    pub fn init_for_connector(
        connector: &crate::ClientConnector,
        server_name: ServerName,
        server_public_key: Vec<u8>,
        kerberos_config: Option<KerberosConfig>,
    ) -> ConnectorResult<(Self, credssp::TsRequest)> {
        let crate::ClientConnectorState::Credssp { selected_protocol } = connector.state else {
            return Err(general_err!("invalid connector state for CredSSP sequence"));
        };
        let (mut sequence, initial_request) = Self::init_with_mode(
            connector.config.credentials.clone(),
            connector.config.domain.as_deref(),
            selected_protocol,
            server_name,
            server_public_key,
            package_for(kerberos_config.as_ref()),
            kerberos_config,
            connector.remote_guard_enabled(),
        )?;
        #[cfg(feature = "remote-guard")]
        if let Some(names) = connector.remote_guard_ntlm() {
            sequence
                .client
                .enable_remote_guard_ntlm(names)
                .map_err(|e| ConnectorError::new("RCG NTLM configuration", ConnectorErrorKind::Credssp(e)))?;
            sequence.remote_guard_ntlm = true;
        }
        #[cfg(not(feature = "remote-guard"))]
        let _ = &mut sequence;
        Ok((sequence, initial_request))
    }

    pub fn remote_guard_enabled(&self) -> bool {
        self.remote_guard
    }

    #[cfg(all(feature = "__test", feature = "remote-guard"))]
    #[doc(hidden)]
    pub fn __test_is_finished(&self) -> bool {
        self.is_finished()
    }

    #[cfg(all(feature = "__test", feature = "remote-guard"))]
    #[doc(hidden)]
    pub fn __test_is_failed(&self) -> bool {
        self.state == CredsspState::Failed
    }

    #[cfg(all(feature = "__test", feature = "remote-guard"))]
    #[doc(hidden)]
    pub fn __test_remote_guard_ntlm_enabled(&self) -> bool {
        self.remote_guard_ntlm_enabled()
    }

    #[cfg(all(feature = "__test", feature = "remote-guard"))]
    #[doc(hidden)]
    pub fn __test_take_remote_guard_session(&mut self) -> ConnectorResult<sspi::rcg::RemoteGuardSession> {
        self.take_remote_guard_session()
    }

    #[cfg(feature = "remote-guard")]
    pub(crate) fn remote_guard_ntlm_enabled(&self) -> bool {
        self.remote_guard_ntlm
    }

    #[cfg(feature = "remote-guard")]
    pub(crate) fn take_remote_guard_session(&mut self) -> ConnectorResult<sspi::rcg::RemoteGuardSession> {
        if self.state != CredsspState::Finished {
            return Err(general_err!("RCG requires a completed CredSSP exchange"));
        }
        self.client
            .take_remote_guard_session()
            .map_err(|e| ConnectorError::new("RCG context", ConnectorErrorKind::Credssp(e)))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the public constructors' inputs plus the Remote Credential Guard mode"
    )]
    fn init_with_mode(
        credentials: Credentials,
        domain: Option<&str>,
        protocol: nego::SecurityProtocol,
        server_name: ServerName,
        server_public_key: Vec<u8>,
        package: CredsspPackage,
        kerberos_config: Option<KerberosConfig>,
        remote_guard: bool,
    ) -> ConnectorResult<(Self, credssp::TsRequest)> {
        if package == CredsspPackage::Ntlm && kerberos_config.is_some() {
            return Err(general_err!(
                "a Kerberos configuration needs the Negotiate or Kerberos package"
            ));
        }
        if remote_guard
            && !matches!(
                kerberos_config.as_ref().map(|cfg| &cfg.kdc_resolution),
                Some(KdcResolution::KdcUrl(_))
            )
        {
            return Err(general_err!("RCG requires an explicit Kerberos configuration"));
        }
        let credentials: sspi::Credentials = match &credentials {
            Credentials::UsernamePassword { username, password } => {
                let username = Username::new(username, domain).map_err(|e| custom_err!("invalid username", e))?;

                sspi::AuthIdentity {
                    username,
                    password: password.to_owned().into(),
                }
                .into()
            }
            Credentials::SmartCard { pin, config } => match config {
                Some(config) => {
                    let cert: Certificate = picky_asn1_der::from_bytes(&config.certificate)
                        .map_err(|_e| general_err!("can't parse certificate"))?;
                    let key = PrivateKey::from_pkcs1(&config.private_key)
                        .map_err(|_e| general_err!("can't parse private key"))?;
                    let identity = sspi::SmartCardIdentity {
                        username: extract_user_principal_name(&cert)
                            .or_else(|| extract_user_name(&cert))
                            .unwrap_or_default(),
                        certificate: cert,
                        reader_name: config.reader_name.clone(),
                        card_name: None,
                        container_name: Some(config.container_name.clone()),
                        csp_name: config.csp_name.clone(),
                        pin: pin.as_bytes().to_vec().into(),
                        private_key: Some(key.into()),
                        scard_type: sspi::SmartCardType::Emulated {
                            scard_pin: Secret::new(pin.as_bytes().to_vec()),
                        },
                    };
                    sspi::Credentials::SmartCard(Box::new(identity))
                }
                None => {
                    return Err(general_err!("smart card configuration missing"));
                }
            },
        };

        let server_name = server_name.into_inner();

        let service_principal_name = format!("TERMSRV/{}", &server_name);

        let client_mode = match package {
            CredsspPackage::Ntlm => credssp::ClientMode::Ntlm(sspi::ntlm::NtlmConfig::default()),
            CredsspPackage::Negotiate | CredsspPackage::Kerberos => {
                // Without a Kerberos configuration, Negotiate starts from NTLM and switches to Kerberos when sspi
                // finds a KDC for the user's realm; with only Kerberos allowed, sspi switches unconditionally.
                let protocol_config: Box<dyn sspi::negotiate::ProtocolConfig> = match kerberos_config {
                    Some(ref krb_config) => Box::new(Into::<sspi::KerberosConfig>::into(krb_config.clone())),
                    None => Box::new(sspi::ntlm::NtlmConfig::default()),
                };
                debug!(?protocol_config, ?package);
                credssp::ClientMode::Negotiate(sspi::NegotiateConfig {
                    protocol_config,
                    package_list: (package == CredsspPackage::Kerberos).then(|| "kerberos".to_owned()),
                    client_computer_name: server_name,
                })
            }
        };

        let mode = credssp::CredSspMode::WithCredentials;
        #[cfg(feature = "remote-guard")]
        let mode = if remote_guard {
            credssp::CredSspMode::RemoteGuard
        } else {
            mode
        };
        let client = CredSspClient::new(
            server_public_key,
            credentials,
            mode,
            client_mode,
            service_principal_name,
        )
        .map_err(|e| ConnectorError::new("CredSSP", ConnectorErrorKind::Credssp(e)))?;

        let sequence = Self {
            client,
            state: CredsspState::Ongoing,
            selected_protocol: protocol,
            remote_guard,
            #[cfg(feature = "remote-guard")]
            remote_guard_ntlm: false,
        };

        let initial_request = credssp::TsRequest::default();

        Ok((sequence, initial_request))
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
                if self.remote_guard {
                    self.state = CredsspState::Failed;
                }
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
            CredsspState::EarlyUserAuthResult if !self.remote_guard => Ok((Written::Nothing, CredsspState::Finished)),
            CredsspState::EarlyUserAuthResult => Err(general_err!("RCG requires the server early-auth result")),
            CredsspState::Finished | CredsspState::Failed => Err(general_err!("CredSSP sequence is already done")),
        }?;

        self.state = next_state;

        Ok(size)
    }
}

fn extract_user_name(cert: &Certificate) -> Option<String> {
    cert.tbs_certificate.subject.find_common_name().map(ToString::to_string)
}

fn extract_user_principal_name(cert: &Certificate) -> Option<String> {
    cert.extensions()
        .iter()
        .find(|ext| ext.extn_id().0 == oids::subject_alternative_name())
        .iter()
        .flat_map(|ext| match ext.extn_value() {
            ExtensionView::SubjectAltName(names) => names.0,
            _ => vec![],
        })
        .find_map(|name| match name {
            GeneralName::OtherName(name) if name.type_id.0 == oids::user_principal_name() => Some(name.value),
            _ => None,
        })
        .and_then(|asn1| picky_asn1_der::from_bytes(&asn1.0.0).ok())
}

fn write_credssp_request(ts_request: credssp::TsRequest, output: &mut WriteBuf) -> ConnectorResult<usize> {
    let length = usize::from(ts_request.buffer_len().map_err(|e| custom_err!("TsRequest", e))?);

    let unfilled_buffer = output.unfilled_to(length);

    ts_request
        .encode_ts_request(unfilled_buffer)
        .map_err(|e| custom_err!("TsRequest", e))?;

    output.advance(length);

    Ok(length)
}
