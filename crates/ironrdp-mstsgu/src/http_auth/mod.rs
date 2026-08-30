//! HTTP authentication helpers for MS-TSGU WebSocket upgrades.
//!
//! Corporate RD Gateways typically challenge with Negotiate and/or NTLM rather than Basic.
//! Negotiate prefers Kerberos (via KDC / `SSPI_KDC_URL`) and falls back to NTLM inside SPNEGO.
//! Pure NTLM is used when the server only offers the NTLM scheme.

#[cfg(all(windows, feature = "native-tls"))]
mod native_http_auth;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sspi::ntlm::NtlmConfig;
use sspi::{
    AuthIdentity, AuthIdentityBuffers, BufferType, ClientRequestFlags, CredentialUse, Credentials, CredentialsBuffers,
    DataRepresentation, InitializeSecurityContextResult, Negotiate, NegotiateConfig, Ntlm, SecurityBuffer,
    SecurityStatus, Sspi as _, SspiImpl as _, Username,
};

use crate::{Error, GwErrorExt as _, GwErrorKind};
#[cfg(all(windows, feature = "native-tls"))]
use native_http_auth::NativeHttpAuth;

/// Result of consuming one HTTP auth challenge/response.
#[derive(Debug)]
pub enum AuthStep {
    /// Send another request with this `Authorization` header value.
    Continue(String),
    /// Authentication finished (caller should inspect the final HTTP status).
    Complete,
    /// Server did not offer NTLM/Negotiate; try HTTP Basic instead.
    TryBasic,
}

// SSPI state objects are large value types; boxing would add an indirection per call.
#[expect(clippy::large_enum_variant, reason = "SSPI state objects are large value types")]
enum AuthBackend {
    Ntlm {
        ntlm: Ntlm,
        credentials_handle: Option<AuthIdentityBuffers>,
    },
    Negotiate {
        negotiate: Negotiate,
        credentials_handle: Option<CredentialsBuffers>,
    },
    #[cfg(all(windows, feature = "native-tls"))]
    Native(NativeHttpAuth),
}

/// Multi-leg HTTP auth state for `Authorization: NTLM …` / `Negotiate …`.
pub struct GatewayHttpAuth {
    backend: AuthBackend,
    /// Scheme used in the Authorization header (`NTLM` or `Negotiate`).
    scheme: &'static str,
    /// Optional SPN / target name (for example `HTTP/rdg.contoso.com`).
    target_name: Option<String>,
    complete: bool,
    allow_basic_fallback: bool,
}

impl GatewayHttpAuth {
    /// Build SSPI state for the MS-TSGU NTLM extended-authentication exchange.
    pub(crate) fn new_extended_auth_ntlm(username: &str, password: &str) -> Result<Self, Error> {
        Self::new_ntlm(username, password, None, None)
    }

    /// Process one NTLM token from the MS-TSGU extended-authentication exchange.
    pub(crate) fn step_extended_auth(&mut self, input: Option<&[u8]>) -> Result<(Vec<u8>, bool), Error> {
        let token = self.initialize(input)?;
        Ok((token, self.complete))
    }

    pub fn scheme(&self) -> &'static str {
        self.scheme
    }

    /// Build auth state from a first-round `WWW-Authenticate` challenge set.
    ///
    /// Prefers Negotiate (Kerberos → NTLM SPNEGO) when advertised, otherwise pure NTLM.
    pub fn from_challenges(
        username: &str,
        password: &str,
        smart_card: Option<&crate::GwSmartCardCredentials>,
        target_name: Option<String>,
        challenges: &[&str],
    ) -> Result<(Option<Self>, AuthStep), Error> {
        Self::from_challenges_with_channel_binding(username, password, smart_card, target_name, None, challenges)
    }

    pub(crate) fn from_challenges_with_channel_binding(
        username: &str,
        password: &str,
        smart_card: Option<&crate::GwSmartCardCredentials>,
        target_name: Option<String>,
        channel_binding: Option<&[u8]>,
        challenges: &[&str],
    ) -> Result<(Option<Self>, AuthStep), Error> {
        let challenges: Vec<&str> = iter_auth_challenges(challenges.iter().copied()).collect();

        #[cfg(feature = "smartcard")]
        if let Some(smart_card) = smart_card {
            let Some(negotiate_token) = challenges
                .iter()
                .find_map(|challenge| split_auth_challenge(challenge, "Negotiate"))
            else {
                return Err(Error::new(
                    "gateway does not offer Negotiate for smart-card authentication",
                    GwErrorKind::UnsupportedFeature,
                ));
            };
            let negotiate_token = if negotiate_token.is_empty() {
                None
            } else {
                Some(
                    STANDARD
                        .decode(negotiate_token.as_bytes())
                        .map_err(|error| Error::custom("decode negotiate challenge", error))?,
                )
            };

            let mut auth = Self::new_negotiate_smartcard(smart_card, target_name)?;
            let token = auth.initialize(negotiate_token.as_deref())?;
            let header = auth.format_authorization(&token);
            return Ok((Some(auth), AuthStep::Continue(header)));
        }

        #[cfg(not(feature = "smartcard"))]
        if smart_card.is_some() {
            return Err(Error::new(
                "smart-card gateway authentication requires the `smartcard` feature",
                GwErrorKind::UnsupportedFeature,
            ));
        }

        let mut saw_negotiate = false;
        let mut saw_ntlm = false;
        let mut saw_basic = false;
        let mut negotiate_token: Option<Vec<u8>> = None;
        let mut ntlm_token: Option<Vec<u8>> = None;

        for value in challenges {
            if let Some(rest) = split_auth_challenge(value, "Negotiate") {
                saw_negotiate = true;
                if rest.is_empty() {
                    if negotiate_token.is_none() {
                        negotiate_token = Some(Vec::new());
                    }
                } else {
                    negotiate_token = Some(
                        STANDARD
                            .decode(rest.as_bytes())
                            .map_err(|e| Error::custom("decode negotiate challenge", e))?,
                    );
                }
            } else if let Some(rest) = split_auth_challenge(value, "NTLM") {
                saw_ntlm = true;
                if rest.is_empty() {
                    if ntlm_token.is_none() {
                        ntlm_token = Some(Vec::new());
                    }
                } else {
                    ntlm_token = Some(
                        STANDARD
                            .decode(rest.as_bytes())
                            .map_err(|e| Error::custom("decode ntlm challenge", e))?,
                    );
                }
            } else if split_auth_challenge(value, "Basic").is_some() {
                saw_basic = true;
            }
        }

        if saw_negotiate {
            let mut auth = Self::new_negotiate(username, password, target_name.clone(), channel_binding)?;
            let input = negotiate_token.as_deref().filter(|t| !t.is_empty());
            // Kerberos SPNEGO can fail without a reachable KDC (for example a gateway SPN
            // with no ticket path); fall back to plain NTLM, which needs no network.
            match auth.initialize(input) {
                Ok(token) => {
                    let header = auth.format_authorization(&token);
                    return Ok((Some(auth), AuthStep::Continue(header)));
                }
                Err(error) if saw_ntlm => {
                    log::debug!("Negotiate failed ({error}); falling back to NTLM");
                }
                Err(error) => return Err(error),
            }
        }

        if saw_ntlm {
            let mut auth = Self::new_ntlm(username, password, target_name, channel_binding)?;
            let input = ntlm_token.as_deref().filter(|t| !t.is_empty());
            let token = auth.initialize(input)?;
            let header = auth.format_authorization(&token);
            return Ok((Some(auth), AuthStep::Continue(header)));
        }

        if saw_basic {
            return Ok((None, AuthStep::TryBasic));
        }

        Err(Error::new(
            "websocket upgrade auth challenge",
            GwErrorKind::UnsupportedFeature,
        ))
    }

    fn new_ntlm(
        username: &str,
        password: &str,
        target_name: Option<String>,
        channel_binding: Option<&[u8]>,
    ) -> Result<Self, Error> {
        #[cfg(not(all(windows, feature = "native-tls")))]
        let _ = channel_binding;
        #[cfg(all(windows, feature = "native-tls"))]
        if let (Some(target_name), Some(channel_binding)) = (target_name.as_deref(), channel_binding) {
            return Self::new_native_http_auth(username, password, target_name, channel_binding, "NTLM");
        }

        let identity = auth_identity(username, password)?;
        let mut ntlm = Ntlm::new();
        let credentials_handle = ntlm
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&identity)
            .execute(&mut ntlm)
            .map_err(|e| Error::custom("acquire ntlm credentials", e))?
            .credentials_handle;

        Ok(Self {
            backend: AuthBackend::Ntlm {
                ntlm,
                credentials_handle,
            },
            scheme: "NTLM",
            target_name,
            complete: false,
            allow_basic_fallback: true,
        })
    }

    fn new_negotiate(
        username: &str,
        password: &str,
        target_name: Option<String>,
        channel_binding: Option<&[u8]>,
    ) -> Result<Self, Error> {
        #[cfg(not(all(windows, feature = "native-tls")))]
        let _ = channel_binding;
        #[cfg(all(windows, feature = "native-tls"))]
        if let (Some(target_name), Some(channel_binding)) = (target_name.as_deref(), channel_binding) {
            return Self::new_native_http_auth(username, password, target_name, channel_binding, "Negotiate");
        }

        let identity = auth_identity(username, password)?;
        let credentials = Credentials::AuthIdentity(identity);
        let client_computer_name = client_computer_name();

        // Start as NTLM; Negotiate upgrades to Kerberos when a KDC is discoverable.
        let config = NegotiateConfig {
            protocol_config: Box::new(NtlmConfig::new(client_computer_name.clone())),
            // Prefer Kerberos when available; keep NTLM fallback; skip PKU2U for gateway HTTP.
            package_list: Some("kerberos,ntlm".to_owned()),
            client_computer_name,
        };

        let mut negotiate = Negotiate::new_client(config).map_err(|e| Error::custom("create negotiate package", e))?;
        let credentials_handle = negotiate
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&credentials)
            .execute(&mut negotiate)
            .map_err(|e| Error::custom("acquire negotiate credentials", e))?
            .credentials_handle;

        Ok(Self {
            backend: AuthBackend::Negotiate {
                negotiate,
                credentials_handle,
            },
            scheme: "Negotiate",
            target_name,
            complete: false,
            allow_basic_fallback: true,
        })
    }

    #[cfg(all(windows, feature = "native-tls"))]
    fn new_native_http_auth(
        username: &str,
        password: &str,
        target_name: &str,
        channel_binding: &[u8],
        package: &'static str,
    ) -> Result<Self, Error> {
        let native = NativeHttpAuth::new(username, password, target_name, channel_binding, package)?;

        Ok(Self {
            backend: AuthBackend::Native(native),
            scheme: package,
            target_name: Some(target_name.to_owned()),
            complete: false,
            allow_basic_fallback: true,
        })
    }

    #[cfg(feature = "smartcard")]
    fn new_negotiate_smartcard(
        smart_card: &crate::GwSmartCardCredentials,
        target_name: Option<String>,
    ) -> Result<Self, Error> {
        use picky::key::PrivateKey;
        use picky_asn1_x509::Certificate;
        use sspi::{KerberosConfig, Secret, SmartCardIdentity, SmartCardType};

        if smart_card.username.is_empty() {
            return Err(Error::new(
                "smart-card username is required",
                GwErrorKind::UnsupportedFeature,
            ));
        }

        let certificate: Certificate = picky_asn1_der::from_bytes(&smart_card.certificate)
            .map_err(|error| Error::custom("parse smart-card certificate", error))?;
        let username = smart_card.username.clone();

        let (private_key, scard_type) = match &smart_card.private_key {
            Some(private_key) => (
                Some(
                    PrivateKey::from_pkcs1(private_key)
                        .map_err(|error| Error::custom("parse smart-card private key", error))?
                        .into(),
                ),
                SmartCardType::Emulated {
                    scard_pin: Secret::new(smart_card.pin.as_bytes().to_vec()),
                },
            ),
            #[cfg(target_os = "windows")]
            None => (None, SmartCardType::WindowsNative),
            #[cfg(not(target_os = "windows"))]
            None => {
                return Err(Error::new(
                    "smart card without a private key requires a Windows card reader",
                    GwErrorKind::UnsupportedFeature,
                ));
            }
        };

        let credentials = Credentials::SmartCard(Box::new(SmartCardIdentity {
            username,
            certificate,
            reader_name: smart_card.reader_name.clone(),
            card_name: smart_card.card_name.clone(),
            container_name: smart_card.container_name.clone(),
            csp_name: smart_card.csp_name.clone().unwrap_or_default(),
            pin: Secret::new(smart_card.pin.as_bytes().to_vec()),
            private_key,
            scard_type,
        }));
        let client_computer_name = client_computer_name();
        let kdc_url = std::env::var("SSPI_KDC_URL").unwrap_or_default();
        let config = NegotiateConfig {
            protocol_config: Box::new(KerberosConfig::new(&kdc_url, client_computer_name.clone())),
            package_list: Some("kerberos".to_owned()),
            client_computer_name,
        };

        let mut negotiate =
            Negotiate::new_client(config).map_err(|error| Error::custom("create negotiate package", error))?;
        let credentials_handle = negotiate
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&credentials)
            .execute(&mut negotiate)
            .map_err(|error| Error::custom("acquire negotiate smart-card credentials", error))?
            .credentials_handle;

        Ok(Self {
            backend: AuthBackend::Negotiate {
                negotiate,
                credentials_handle,
            },
            scheme: "Negotiate",
            target_name,
            complete: false,
            allow_basic_fallback: false,
        })
    }

    /// Consume a subsequent `WWW-Authenticate` challenge and produce the next step.
    pub(crate) fn step_www_authenticate<'a, I>(&mut self, challenges: I) -> Result<AuthStep, Error>
    where
        I: IntoIterator<Item = &'a str>,
    {
        if self.complete {
            return Ok(AuthStep::Complete);
        }

        let mut saw_basic = false;
        let mut token: Option<Vec<u8>> = None;

        for value in iter_auth_challenges(challenges) {
            let preferred = match self.scheme {
                "Negotiate" => split_auth_challenge(value, "Negotiate"),
                _ => split_auth_challenge(value, "NTLM").or_else(|| split_auth_challenge(value, "Negotiate")),
            };

            if let Some(rest) = preferred {
                if rest.is_empty() {
                    if token.is_none() {
                        token = Some(Vec::new());
                    }
                } else {
                    token = Some(
                        STANDARD
                            .decode(rest.as_bytes())
                            .map_err(|e| Error::custom("decode auth challenge", e))?,
                    );
                }
            } else if split_auth_challenge(value, "Basic").is_some() {
                saw_basic = true;
            }
        }

        match token {
            Some(raw) => {
                let input = if raw.is_empty() { None } else { Some(raw.as_slice()) };
                let next = self.initialize(input)?;
                if self.complete && next.is_empty() {
                    Ok(AuthStep::Complete)
                } else {
                    Ok(AuthStep::Continue(self.format_authorization(&next)))
                }
            }
            None if saw_basic && self.allow_basic_fallback => Ok(AuthStep::TryBasic),
            None => Err(Error::new(
                "websocket upgrade auth challenge",
                GwErrorKind::UnsupportedFeature,
            )),
        }
    }

    /// Consume a final `WWW-Authenticate` token on a successful upgrade (RFC 4559).
    ///
    /// Missing tokens are accepted: NTLM upgrades typically omit them, and some
    /// Negotiate servers do not send an AP-REP. A present token must complete SSPI.
    pub(crate) fn finish_www_authenticate<'a, I>(&mut self, challenges: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = &'a str>,
    {
        if self.complete {
            return Ok(());
        }

        let mut token: Option<Vec<u8>> = None;
        for value in iter_auth_challenges(challenges) {
            let rest = match self.scheme {
                "Negotiate" => split_auth_challenge(value, "Negotiate"),
                _ => split_auth_challenge(value, "NTLM").or_else(|| split_auth_challenge(value, "Negotiate")),
            };
            if let Some(rest) = rest.filter(|rest| !rest.is_empty()) {
                token = Some(
                    STANDARD
                        .decode(rest.as_bytes())
                        .map_err(|e| Error::custom("decode auth challenge", e))?,
                );
            }
        }

        if let Some(raw) = token {
            let _ = self.initialize(Some(&raw))?;
            if !self.complete {
                return Err(Error::new(
                    "websocket upgrade mutual auth incomplete",
                    GwErrorKind::Connect,
                ));
            }
        }

        Ok(())
    }

    fn format_authorization(&self, token: &[u8]) -> String {
        format!("{} {}", self.scheme, STANDARD.encode(token))
    }

    fn initialize(&mut self, input: Option<&[u8]>) -> Result<Vec<u8>, Error> {
        let mut input_token = [SecurityBuffer::new(
            input.map(<[u8]>::to_vec).unwrap_or_default(),
            BufferType::Token,
        )];
        let mut output_token = [SecurityBuffer::new(Vec::with_capacity(1024), BufferType::Token)];

        let status = match &mut self.backend {
            AuthBackend::Ntlm {
                ntlm,
                credentials_handle,
            } => {
                let mut builder = ntlm
                    .initialize_security_context()
                    .with_credentials_handle(credentials_handle)
                    .with_context_requirements(default_context_flags())
                    .with_target_data_representation(DataRepresentation::Native)
                    .with_input(&mut input_token)
                    .with_output(&mut output_token);
                builder.target_name = self.target_name.as_deref();

                let InitializeSecurityContextResult { status, .. } = ntlm
                    .initialize_security_context_impl(&mut builder)
                    .map_err(|e| Error::custom("ntlm initialize security context", e))?
                    .resolve_to_result()
                    .map_err(|e| Error::custom("ntlm initialize security context", e))?;
                status
            }
            AuthBackend::Negotiate {
                negotiate,
                credentials_handle,
            } => {
                let mut builder = negotiate
                    .initialize_security_context()
                    .with_credentials_handle(credentials_handle)
                    .with_context_requirements(default_context_flags())
                    .with_target_data_representation(DataRepresentation::Native)
                    .with_input(&mut input_token)
                    .with_output(&mut output_token);
                builder.target_name = self.target_name.as_deref();

                let mut generator = negotiate
                    .initialize_security_context_impl(&mut builder)
                    .map_err(|e| Error::custom("negotiate initialize security context", e))?;

                // Kerberos may suspend for KDC traffic; NTLM completes without network.
                let InitializeSecurityContextResult { status, .. } = generator
                    .resolve_with_default_network_client()
                    .map_err(|e| Error::custom("negotiate initialize security context", e))?;
                status
            }
            #[cfg(all(windows, feature = "native-tls"))]
            AuthBackend::Native(native) => {
                let step = native.initialize(input)?;
                self.complete = step.complete;
                return Ok(step.token);
            }
        };

        match status {
            SecurityStatus::Ok => {
                self.complete = true;
            }
            SecurityStatus::ContinueNeeded => {}
            other => {
                return Err(Error::new("http auth security status", GwErrorKind::Connect)
                    .with_source(std::io::Error::other(format!("unexpected auth status: {other:?}"))));
            }
        }

        Ok(core::mem::take(&mut output_token[0].buffer))
    }
}

/// Build a Basic authorization header value.
pub fn basic_authorization(username: &str, password: &str) -> String {
    let token = STANDARD.encode(format!("{username}:{password}"));
    format!("Basic {token}")
}

fn auth_identity(username: &str, password: &str) -> Result<AuthIdentity, Error> {
    let username = Username::parse(username)
        .or_else(|_| Username::new(username, None))
        .map_err(|e| Error::custom("parse gateway username", e))?;

    Ok(AuthIdentity {
        username,
        password: password.to_owned().into(),
    })
}

fn client_computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "IRONRDP".to_owned())
}

fn default_context_flags() -> ClientRequestFlags {
    ClientRequestFlags::ALLOCATE_MEMORY
        | ClientRequestFlags::CONFIDENTIALITY
        | ClientRequestFlags::INTEGRITY
        | ClientRequestFlags::MUTUAL_AUTH
}

fn iter_auth_challenges<'a, I>(values: I) -> impl Iterator<Item = &'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    values.into_iter().flat_map(split_challenge_list)
}

/// Split a list-valued `WWW-Authenticate` field into individual challenges.
///
/// Commas inside quoted auth-param values are not separators.
fn split_challenge_list(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                if let Some(piece) = value.get(start..i).map(str::trim).filter(|piece| !piece.is_empty()) {
                    out.push(piece);
                }
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    if let Some(piece) = value.get(start..).map(str::trim).filter(|piece| !piece.is_empty()) {
        out.push(piece);
    }
    out
}

/// Case-insensitive scheme match; returns the remainder after the scheme (may be empty).
pub fn split_auth_challenge<'a>(header_value: &'a str, scheme: &str) -> Option<&'a str> {
    let header_value = header_value.trim();
    let scheme_len = scheme.len();
    if header_value.len() < scheme_len {
        return None;
    }
    if !header_value.as_bytes()[..scheme_len].eq_ignore_ascii_case(scheme.as_bytes()) {
        return None;
    }
    // The scheme is ASCII, so this byte offset is always a char boundary.
    let rest = header_value.get(scheme_len..)?;
    if rest.is_empty() {
        return Some("");
    }
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some(rest.trim())
}

/// Collect all `WWW-Authenticate` header values as strings.
pub(crate) fn www_authenticate_values(headers: &hyper::HeaderMap) -> Vec<&str> {
    headers
        .get_all(hyper::header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect()
}
